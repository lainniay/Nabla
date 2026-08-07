import type {
  AgentSessionRuntime,
  InlineExtension,
  ModelRuntime,
  ToolDefinition,
} from "@earendil-works/pi-coding-agent";

import { modelReference, type AgentProfile } from "./profile-model.ts";
import type { HarnessConfig } from "../workspace/config.ts";
import { MUTATING_TOOL_NAMES } from "../permissions/shell/rules.ts";
import type {
  ActiveAgentSnapshot,
  PendingIntegrationSnapshot,
  WorktreeIntegrationSnapshot,
} from "../../protocol/contracts.ts";
import type { JsonObject } from "../../protocol/validation.ts";
import type { RuntimeSupervisor } from "../../runtime/runtime-supervisor.ts";
import type { PlanModePort } from "../plans/plan-controller.ts";
import type {
  WorktreeRecord,
  WorktreeRecoveryState,
} from "./isolation/worktree.ts";
import type { WorkspaceService } from "../workspace/workspace-service.ts";
import type { PermissionService } from "../permissions/permission-service.ts";
import type { ToolAuthorizationContext } from "../permissions/permission-service.ts";
import type { IntegrationService } from "./integration-service.ts";
import type { RustSandboxBackend } from "../permissions/execution/rust-sandbox-backend.ts";
import { createNablaBashTool } from "../../runtime/create-nabla-bash-tool.ts";
import { buildWorkspaceContext } from "../../runtime/workspace-context.ts";
import { normalizeToolInputPaths } from "../permissions/filesystem/path.ts";
import {
  SubagentRunner,
  type SubagentRunnerPort,
} from "./subagent-runner.ts";
import type {
  ActiveSubagent,
  CompletedSubagent,
  SubagentHandle,
  SubagentOptions,
} from "./subagent-types.ts";

export class SubagentSupervisor implements SubagentRunnerPort {
  private readonly subagents = new Map<string, ActiveSubagent>();
  private readonly completedSubagents = new Map<string, CompletedSubagent>();
  private readonly runner: SubagentRunner;
  private sequence = 0;
  private writeTail: Promise<unknown> = Promise.resolve();
  private readonly workspace: WorkspaceService;
  private readonly integrations: IntegrationService;
  private readonly permissions: PermissionService;
  private readonly sandboxBackend: RustSandboxBackend;
  private readonly modelRuntime: ModelRuntime;
  private readonly runtime: RuntimeSupervisor;
  private readonly planMode: PlanModePort;
  private readonly sendEvent: (event: JsonObject) => void;
  private readonly warn: (message: string) => void;
  private readonly onAgentsChanged: () => void;

  constructor(
    workspace: WorkspaceService,
    integrations: IntegrationService,
    permissions: PermissionService,
    sandboxBackend: RustSandboxBackend,
    modelRuntime: ModelRuntime,
    runtime: RuntimeSupervisor,
    planMode: PlanModePort,
    sendEvent: (event: JsonObject) => void,
    warn: (message: string) => void,
    onAgentsChanged: () => void,
  ) {
    this.workspace = workspace;
    this.integrations = integrations;
    this.permissions = permissions;
    this.sandboxBackend = sandboxBackend;
    this.modelRuntime = modelRuntime;
    this.runtime = runtime;
    this.planMode = planMode;
    this.sendEvent = sendEvent;
    this.warn = warn;
    this.onAgentsChanged = onAgentsChanged;
    this.runner = new SubagentRunner(integrations, modelRuntime, this);
  }

  start(input: {
    profile: string;
    task: string;
  }): { accepted: boolean; agent: ActiveAgentSnapshot } {
    const handle = this.launch({
      profile: input.profile,
      task: input.task,
      direct: true,
    });
    void handle.completion.catch(() => undefined);
    return {
      accepted: true,
      agent: this.publicSubagent(handle.agent),
    };
  }

  async cancel(agentId: string): Promise<void> {
    const agent = this.subagents.get(agentId);
    if (!agent) throw new Error(`Subagent is not running: ${agentId}`);
    agent.controller.abort();
    if (agent.session) await agent.session.abort();
  }

  async integrate(input: {
    agentId: string;
    action: "apply" | "resolve" | "keep" | "discard";
  }): Promise<JsonObject> {
    const { agentId, action } = input;
    const completed = this.completedSubagents.get(agentId);
    if (!completed) {
      throw new Error(`Subagent has no pending worktree result: ${agentId}`);
    }
    let record = completed.record;
    let integrationWarning: string | undefined;
    if (action === "resolve") {
      const handle = await this.resolvePending(agentId);
      void handle.completion.catch((error) => {
        this.restoreResolutionFailure(agentId, error);
      });
      return {
        status: "resolving",
        resolver: this.publicSubagent(handle.agent),
      };
    }
    if (action === "keep") {
      record = await this.integrations.keep(record);
      this.completedSubagents.delete(agentId);
    } else if (action === "discard") {
      record = await this.integrations.discard(record);
      this.completedSubagents.delete(agentId);
    } else {
      const result = await this.integrations.integrate(record);
      record = result.record;
      if (result.status !== "applied") {
        completed.record = record;
        completed.agent.integrationStatus = record.integrationStatus;
        this.sendEvent({
          type: "subagent_integration",
          event: record.integrationStatus,
          agent: this.publicSubagent(completed.agent),
          integration: this.worktreeSummary(completed.agent),
          error: result.error,
        });
        this.onAgentsChanged();
        return {
          status: record.integrationStatus,
          integration: this.worktreeSummary(completed.agent),
        };
      }
      integrationWarning = result.error;
      completed.agent.integrationStatus = "applied";
      this.completedSubagents.delete(agentId);
    }
    completed.record = record;
    completed.agent.integrationStatus = record.integrationStatus;
    this.sendEvent({
      type: "subagent_integration",
      event: record.integrationStatus,
      agent: this.publicSubagent(completed.agent),
      integration: this.worktreeSummary(completed.agent),
      ...(integrationWarning ? { error: integrationWarning } : {}),
    });
    this.onAgentsChanged();
    return {
      status: record.integrationStatus,
      integration: this.worktreeSummary(completed.agent),
      ...(integrationWarning ? { warning: integrationWarning } : {}),
    };
  }

  run(options: SubagentOptions): Promise<JsonObject> {
    return this.launch(options).completion;
  }

  hostClose(): void {
    const activeSubagents = [...this.subagents.values()];
    for (const subagent of activeSubagents) subagent.controller.abort();
    void Promise.allSettled(
      activeSubagents.flatMap((subagent) =>
        subagent.session ? [subagent.session.abort()] : [],
      ),
    );
  }

  restoreRecovered(
    active: ActiveSubagent,
    result: JsonObject,
    record: WorktreeRecord,
  ): void {
    this.completedSubagents.set(active.id, {
      agent: active,
      result,
      record,
    });
    const sequence = /^agent-(\d+)$/u.exec(record.agentId)?.[1];
    if (sequence) {
      this.sequence = Math.max(this.sequence, Number.parseInt(sequence, 10));
    }
  }

  activeSnapshots(): ActiveAgentSnapshot[] {
    return [...this.subagents.values()].map((agent) =>
      this.publicSubagent(agent),
    );
  }

  pendingSnapshots(): ActiveAgentSnapshot[] {
    return [...this.completedSubagents.values()].map(({ agent }) =>
      this.publicSubagent(agent),
    );
  }

  pendingIntegrations(): PendingIntegrationSnapshot[] {
    return [...this.completedSubagents.values()].map(({ agent }) => ({
      agent: this.publicSubagent(agent),
      integration: this.worktreeSummary(agent),
    }));
  }

  private launch(options: SubagentOptions): SubagentHandle {
    const profile = this.workspace.configValue().profiles[options.profile];
    if (!profile) {
      throw new Error(`Unknown agent profile: ${options.profile}`);
    }
    if (profile.disabled) {
      throw new Error(`Subagent profile is disabled: ${options.profile}`);
    }
    const unavailable = this.workspace.profileUnavailableReason(profile);
    if (unavailable) throw new Error(`Subagent ${options.profile}: ${unavailable}`);
    if (this.subagents.size >= this.workspace.configValue().maxParallel) {
      throw new Error(
        `Subagent concurrency limit reached (${this.workspace.configValue().maxParallel})`,
      );
    }
    const activeForProfile = [...this.subagents.values()].filter(
      (agent) => agent.profile === options.profile,
    ).length;
    if (activeForProfile >= profile.maxParallel) {
      throw new Error(
        `Profile ${options.profile} concurrency limit reached (${profile.maxParallel})`,
      );
    }
    const runtime = this.runtime.current();
    const cwd = runtime.session.sessionManager.getCwd();
    const modelRef = modelReference(profile);
    const model = modelRef
      ? this.modelRuntime.getModel(modelRef.provider, modelRef.id)
      : runtime.session.model;
    if (!model) {
      throw new Error(
        modelRef
          ? `Configured subagent model is unavailable: ${modelRef.provider}/${modelRef.id}`
          : "No model is selected for the subagent",
      );
    }
    const agentId = `agent-${++this.sequence}`;
    const controller = new AbortController();
    const abortFromParent = () => controller.abort();
    if (options.parentSignal?.aborted) {
      controller.abort();
    } else {
      options.parentSignal?.addEventListener("abort", abortFromParent, {
        once: true,
      });
    }
    const active: ActiveSubagent = {
      id: agentId,
      profile: options.profile,
      task: options.task,
      direct: options.direct === true,
      planReadOnly: this.planMode.current(),
      lifecycle: "queued",
      originSession: runtime.session,
      originSessionId: runtime.session.sessionId,
      controller,
      startedAt: new Date().toISOString(),
      turns: 0,
      maxTurns: profile.maxTurns,
      model: `${model.provider}/${model.id}`,
      isolationBackend: "shared",
      integrationStatus: "none",
    };
    this.subagents.set(agentId, active);
    this.sendEvent({
      type: "subagent_state",
      event: "queued",
      agent: this.publicSubagent(active),
    });
    this.onAgentsChanged();

    const run = async () => {
      active.lifecycle = "preparing_isolation";
      this.sendEvent({
        type: "subagent_state",
        event: "preparing_isolation",
        agent: this.publicSubagent(active),
      });
      const prepared =
        options.preparedIsolation ??
        (await this.integrations.prepare(
          active.id,
          cwd,
          profile.isolation,
          controller.signal,
        ));
      active.isolationBackend = prepared.backend;
      active.isolationWarning = prepared.warning;
      active.worktree = prepared.record;
      if (active.worktree) {
        active.worktree = await this.integrations.annotate(
          active.worktree,
          this.worktreeRecoveryState(active),
        );
      }
      this.sendEvent({
        type: "subagent_state",
        event:
          prepared.backend === "worktree" ? "isolated" : prepared.backend,
        agent: this.publicSubagent(active),
        ...(prepared.warning ? { warning: prepared.warning } : {}),
      });
      const execute = () =>
        this.runner.run(
          active,
          options,
          structuredClone(profile),
          model,
          prepared.executionCwd,
          cwd,
        );
      const writeCapable = profile.tools.some((tool) =>
        MUTATING_TOOL_NAMES.has(tool),
      );
      if (writeCapable && prepared.backend !== "worktree") {
        const sharedExecution = this.writeTail.then(execute, execute);
        this.writeTail = sharedExecution.catch(() => undefined);
        return sharedExecution;
      }
      return execute();
    };
    const execution = run().catch((error) => {
      if (this.subagents.has(active.id)) {
        this.finishSubagent(
          active,
          active.controller.signal.aborted ? "cancelled" : "failed",
          undefined,
          error instanceof Error ? error.message : String(error),
        );
        this.subagents.delete(active.id);
        this.onAgentsChanged();
      }
      throw error;
    });
    const cancelled = new Promise<never>((_resolve, reject) => {
      const rejectCancelled = () => {
        if (
          active.lifecycle === "queued" &&
          this.subagents.delete(active.id)
        ) {
          this.finishSubagent(
            active,
            "cancelled",
            undefined,
            "Subagent cancelled",
          );
          this.onAgentsChanged();
        }
        reject(new Error("Subagent cancelled"));
      };
      if (controller.signal.aborted) rejectCancelled();
      else {
        controller.signal.addEventListener("abort", rejectCancelled, {
          once: true,
        });
      }
      execution.finally(() => {
        controller.signal.removeEventListener("abort", rejectCancelled);
      }).catch(() => undefined);
    });
    const completion = Promise.race([execution, cancelled]).finally(() => {
      options.parentSignal?.removeEventListener("abort", abortFromParent);
    });
    return { agent: active, completion };
  }

  private async resolvePending(agentId: string): Promise<SubagentHandle> {
    const completed = this.completedSubagents.get(agentId);
    if (!completed) {
      throw new Error(`Subagent has no pending worktree result: ${agentId}`);
    }
    completed.agent.lifecycle = "resolving";
    this.sendEvent({
      type: "subagent_integration",
      event: "resolving",
      agent: this.publicSubagent(completed.agent),
      integration: this.worktreeSummary(completed.agent),
    });
    let prepared;
    try {
      prepared = await this.integrations.prepareResolution(
        `${agentId}-resolver`,
        completed.record,
      );
    } catch (error) {
      this.restoreResolutionFailure(agentId, error);
      throw error;
    }
    const conflictContext = [
      `Resolve integration conflicts for subagent ${agentId}.`,
      `Original task:\n${completed.agent.task}`,
      `Original result:\n${JSON.stringify(completed.result)}`,
      `Changed paths:\n${completed.record.changedPaths.map((path) => `- ${path}`).join("\n")}`,
      prepared.conflictPaths.length > 0
        ? `Conflicted paths:\n${prepared.conflictPaths.map((path) => `- ${path}`).join("\n")}`
        : "Git completed a three-way merge. Inspect and verify the merged result.",
      prepared.diagnostic
        ? `Git diagnostic:\n${prepared.diagnostic}`
        : "",
      "Work only inside the supplied integration workspace. Preserve both the current workspace changes and the original task intent. Remove all conflict markers and run relevant verification.",
    ]
      .filter(Boolean)
      .join("\n\n");
    try {
      return this.launch({
        profile: completed.agent.profile,
        task: conflictContext,
        direct: true,
        preparedIsolation: prepared.isolation,
        forceAutoIntegrate: true,
        resolutionForAgentId: agentId,
      });
    } catch (error) {
      try {
        await this.integrations.discard(prepared.isolation.record);
      } catch (cleanupError) {
        this.warn(
          `Unable to discard failed conflict resolver ${prepared.isolation.record.id}: ${
            cleanupError instanceof Error
              ? cleanupError.message
              : String(cleanupError)
          }`,
        );
      }
      this.restoreResolutionFailure(agentId, error);
      throw error;
    }
  }

  private restoreResolutionFailure(agentId: string, error: unknown): void {
    const pending = this.completedSubagents.get(agentId);
    if (!pending) return;
    if (pending.agent.lifecycle === "awaiting_integration") return;
    pending.agent.lifecycle = "awaiting_integration";
    pending.agent.integrationStatus = pending.record.integrationStatus;
    this.sendEvent({
      type: "subagent_integration",
      event: "conflicted",
      agent: this.publicSubagent(pending.agent),
      integration: this.worktreeSummary(pending.agent),
      error: error instanceof Error ? error.message : String(error),
    });
  }

  // SubagentRunnerPort implementation
  send(event: JsonObject): void {
    this.sendEvent(event);
  }

  publishAgentsState(): void {
    this.onAgentsChanged();
  }

  removeActive(agentId: string): void {
    this.subagents.delete(agentId);
  }

  markPendingIntegration(
    agentId: string,
    active: ActiveSubagent,
    result: JsonObject,
    record: WorktreeRecord,
  ): void {
    this.completedSubagents.set(agentId, {
      agent: active,
      result,
      record,
    });
  }

  resolveSource(agentId: string): CompletedSubagent | undefined {
    return this.completedSubagents.get(agentId);
  }

  deleteCompleted(agentId: string): void {
    this.completedSubagents.delete(agentId);
  }

  finishSubagent(
    active: ActiveSubagent,
    event:
      | "completed"
      | "awaiting_integration"
      | "limit_reached"
      | "failed"
      | "cancelled",
    result?: JsonObject,
    error?: string,
  ): void {
    this.sendEvent({
      type: "subagent_state",
      event,
      agent: this.publicSubagent(active),
      ...(result ? { result } : {}),
      ...(error ? { error } : {}),
    });
  }

  async injectDirectSubagentResult(
    active: ActiveSubagent,
    result: JsonObject,
  ): Promise<void> {
    const summary =
      typeof result.summary === "string"
        ? result.summary
        : "Subagent completed the assigned task.";
    await active.originSession.sendCustomMessage(
      {
        customType: "nabla.subagent-result.v1",
        display: false,
        content: [
          `Subagent ${active.id} (${active.profile}) result.`,
          `Task: ${active.task}`,
          `Status: ${String(result.status ?? "completed")}`,
          `Summary: ${summary.slice(0, 8_000)}`,
        ].join("\n"),
        details: {
          agentId: active.id,
          profile: active.profile,
          task: active.task,
          model: active.model,
          result,
        },
      },
      { triggerTurn: false },
    );
  }

  subagentExtension(
    agentId: string,
    profileName: string,
    profile: AgentProfile,
    model: string,
  ): InlineExtension {
    return {
      name: `nabla-subagent-${agentId}`,
      factory: (pi) => {
        pi.on("before_agent_start", (event, context) => ({
          systemPrompt: [
            event.systemPrompt,
            buildWorkspaceContext(context.cwd),
            `This is independent subagent ${agentId} (${profileName}).`,
            ...profile.instructions,
            "Do not ask the user directly. Return structured results to the parent agent.",
          ].join("\n\n"),
        }));
        pi.on("tool_call", (event, context) => {
          if (event.toolName === "bash") return;
          normalizeToolInputPaths(
            event.input as Record<string, unknown>,
            context.cwd,
          );
          return this.permissions.authorizeTool(event, {
            cwd: context.cwd,
            signal: context.signal,
            agent: {
              agentId,
              profile: profileName,
              model,
              profileConfig: profile,
              planReadOnly:
                this.subagents.get(agentId)?.planReadOnly === true,
              sessionId: context.sessionManager.getSessionId(),
            },
          });
        });
        pi.on("tool_result", (event) => {
          if (event.toolName === "bash") return;
          this.permissions.finishTool(event.toolCallId, !event.isError);
        });
      },
    };
  }

  publicSubagent(agent: ActiveSubagent): ActiveAgentSnapshot {
    return {
      id: agent.id,
      profile: agent.profile,
      task: agent.task,
      lifecycle: agent.lifecycle,
      startedAt: agent.startedAt,
      turns: agent.turns,
      maxTurns: agent.maxTurns,
      model: agent.model,
      originSessionId: agent.originSessionId,
      isolationBackend: agent.isolationBackend,
      integrationStatus: agent.integrationStatus,
      isolationWarning: agent.isolationWarning ?? null,
    };
  }

  worktreeSummary(agent: ActiveSubagent): WorktreeIntegrationSnapshot {
    const record = agent.worktree;
    return {
      backend: agent.isolationBackend,
      status: agent.integrationStatus,
      warning: agent.isolationWarning ?? null,
      artifactId: record?.id ?? null,
      changedPaths: record?.changedPaths ?? [],
      patchBytes: record?.patchBytes ?? 0,
      excludedPaths: record?.excludedPaths ?? [],
      resolverAvailable: (record?.resolutionAttempts ?? 0) < 1,
    };
  }

  worktreeRecoveryState(
    agent: ActiveSubagent,
    result?: JsonObject,
  ): WorktreeRecoveryState {
    return {
      profile: agent.profile,
      task: agent.task,
      direct: agent.direct,
      planReadOnly: agent.planReadOnly,
      model: agent.model,
      originSessionId: agent.originSessionId,
      ...(result ? { result: structuredClone(result) } : {}),
    };
  }

  reportHostWarning(message: string): void {
    this.warn(message);
    this.sendEvent({ type: "host_warning", message });
  }

  workspaceConfig(): HarnessConfig {
    return this.workspace.configValue();
  }

  createBashTool(
    cwd: string,
    agent: ToolAuthorizationContext["agent"],
  ): ToolDefinition {
    return createNablaBashTool(cwd, {
      permissions: this.permissions,
      sandboxBackend: this.sandboxBackend,
      agent,
    });
  }
}
