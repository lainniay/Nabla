import {
  DefaultResourceLoader,
  SessionManager,
  SettingsManager,
  createAgentSession,
  getAgentDir,
  type AgentSession,
  type InlineExtension,
  type ModelRuntime,
  type ToolDefinition,
} from "@earendil-works/pi-coding-agent";

import {
  filterContextFilesByTrust,
  workspaceIsTrusted,
} from "../workspace/trust.ts";
import type { HarnessConfig } from "../workspace/config.ts";
import {
  type AgentProfile,
} from "./profile-model.ts";
import { parseSubagentOutput } from "../../protocol/subagent-output.ts";
import { isJsonObject, type JsonObject } from "../../protocol/validation.ts";
import type {
  ActiveAgentSnapshot,
  WorktreeIntegrationSnapshot,
} from "../../protocol/contracts.ts";
import type {
  WorktreeRecoveryState,
  WorktreeRecord,
} from "./isolation/worktree.ts";
import type {
  ActiveSubagent,
  CompletedSubagent,
  SubagentOptions,
} from "./subagent-types.ts";
import type { IntegrationService } from "./integration-service.ts";
import type { ToolAuthorizationContext } from "../permissions/permission-service.ts";

export interface SubagentRunnerPort {
  send(event: JsonObject): void;
  publishAgentsState(): void;
  removeActive(agentId: string): void;
  markPendingIntegration(
    agentId: string,
    active: ActiveSubagent,
    result: JsonObject,
    record: WorktreeRecord,
  ): void;
  resolveSource(agentId: string): CompletedSubagent | undefined;
  deleteCompleted(agentId: string): void;
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
  ): void;
  injectDirectSubagentResult(
    active: ActiveSubagent,
    result: JsonObject,
  ): Promise<void>;
  subagentExtension(
    agentId: string,
    profileName: string,
    profile: AgentProfile,
    modelId: string,
  ): InlineExtension;
  publicSubagent(active: ActiveSubagent): ActiveAgentSnapshot;
  worktreeSummary(active: ActiveSubagent): WorktreeIntegrationSnapshot;
  worktreeRecoveryState(
    active: ActiveSubagent,
    result?: JsonObject,
  ): WorktreeRecoveryState;
  reportHostWarning(message: string): void;
  workspaceConfig(): HarnessConfig;
  createBashTool(
    cwd: string,
    agent: ToolAuthorizationContext["agent"],
  ): ToolDefinition;
}

export class SubagentRunner {
  private readonly integrations: IntegrationService;
  private readonly modelRuntime: ModelRuntime;
  private readonly port: SubagentRunnerPort;

  constructor(
    integrations: IntegrationService,
    modelRuntime: ModelRuntime,
    port: SubagentRunnerPort,
  ) {
    this.integrations = integrations;
    this.modelRuntime = modelRuntime;
    this.port = port;
  }

  async run(
    active: ActiveSubagent,
    options: SubagentOptions,
    profile: AgentProfile,
    model: NonNullable<AgentSession["model"]>,
    cwd: string,
    originCwd: string,
  ): Promise<JsonObject> {
    if (active.controller.signal.aborted) {
      throw new Error("Subagent cancelled");
    }
    const controller = active.controller;
    const agentId = active.id;
    const settings = SettingsManager.inMemory();
    settings.setProjectTrusted(
      workspaceIsTrusted(originCwd, this.port.workspaceConfig()),
    );
    const loader = new DefaultResourceLoader({
      cwd,
      agentDir: getAgentDir(),
      settingsManager: settings,
      noThemes: true,
      noExtensions: true,
      agentsFilesOverride: (base) => ({
        agentsFiles: filterContextFilesByTrust(
          base.agentsFiles,
          getAgentDir(),
          workspaceIsTrusted(originCwd, this.port.workspaceConfig()),
        ),
      }),
      skillsOverride: (base) => ({
        ...base,
        skills: base.skills.filter((skill) =>
          profile.skills.includes(skill.name),
        ),
      }),
      extensionFactories: [
        this.port.subagentExtension(agentId, options.profile, profile, model.id),
      ],
    });
    await loader.reload({
      resolveProjectTrust: async () =>
        workspaceIsTrusted(originCwd, this.port.workspaceConfig()),
    });
    const result = await createAgentSession({
      cwd,
      agentDir: getAgentDir(),
      modelRuntime: this.modelRuntime,
      model,
      thinkingLevel:
        profile.thinkingLevel ?? active.originSession.thinkingLevel,
      tools: profile.tools,
      resourceLoader: loader,
      sessionManager: SessionManager.inMemory(cwd),
      settingsManager: settings,
      customTools: [
        this.port.createBashTool(cwd, {
          agentId: active.id,
          profile: options.profile,
          model: `${model.provider}/${model.id}`,
          profileConfig: profile,
          planReadOnly: active.planReadOnly,
        }),
      ],
    });
    const session = result.session;
    active.session = session;
    active.lifecycle = "running";
    this.port.send({
      type: "subagent_state",
      event: "started",
      agent: this.port.publicSubagent(active),
    });
    this.port.publishAgentsState();

    let finalMessages: unknown[] = [];
    let limitReached = false;
    const unsubscribe = session.subscribe((event) => {
      if (event.type === "agent_end") finalMessages = event.messages;
      if (event.type === "turn_start") {
        if (active.turns >= active.maxTurns) {
          limitReached = true;
          controller.abort();
        } else {
          active.turns += 1;
        }
      }
    });
    const abortChild = () => void session.abort();
    controller.signal.addEventListener("abort", abortChild, { once: true });
    try {
      const prompt = [
        `You are Nabla subagent ${agentId} using profile ${options.profile}.`,
        `Assigned task:\n${options.task}`,
        "Return one JSON object only: {status, summary, evidence, changedPaths, verification, blockers}.",
      ]
        .filter(Boolean)
        .join("\n\n");
      await session.prompt(prompt);
      if (controller.signal.aborted) throw new Error("Subagent cancelled");
      const text = lastAssistantText(finalMessages);
      const parsed = parseSubagentOutput(text);
      let completed: JsonObject = {
        ...parsed,
        agentId,
        profile: options.profile,
        model: `${model.provider}/${model.id}`,
      };
      let integrationPending = false;
      if (active.worktree) {
        const captured = await this.integrations.capture(
          active.worktree,
          controller.signal,
        );
        active.worktree = captured.record;
        this.integrations.validateWorktreePaths(
          captured.record,
          profile,
          originCwd,
        );
        if (options.resolutionForAgentId) {
          await this.integrations.assertResolved(captured.record);
        }
        if (options.discardWorktreeChanges) {
          if (captured.hasChanges) {
            active.worktree = await this.integrations.discard(captured.record);
            active.integrationStatus = "discarded";
            throw new Error(
              `Verification modified isolated files: ${captured.record.changedPaths.join(", ")}`,
            );
          }
          const integration = await this.integrations.integrate(captured.record);
          active.worktree = integration.record;
          active.integrationStatus = integration.record.integrationStatus;
          if (integration.status !== "applied") {
            throw new Error(
              integration.error ?? "Unable to close the verification worktree",
            );
          }
          if (integration.error) this.port.reportHostWarning(integration.error);
        }
        const autoIntegrate =
          !options.discardWorktreeChanges &&
          (!captured.hasChanges ||
            (options.forceAutoIntegrate === true ||
              profile.isolation.integration === "auto"));
        if (autoIntegrate) {
          const integration = await this.integrations.integrate(
            captured.record,
            controller.signal,
          );
          active.worktree = integration.record;
          active.integrationStatus = integration.record.integrationStatus;
          if (integration.status !== "applied") {
            integrationPending = true;
            active.lifecycle = "awaiting_integration";
            active.worktree = await this.integrations.annotate(
              integration.record,
              this.port.worktreeRecoveryState(active, completed),
            );
            this.port.markPendingIntegration(
              agentId,
              active,
              completed,
              active.worktree,
            );
          } else if (integration.error) {
            this.port.reportHostWarning(integration.error);
          }
        } else {
          integrationPending = captured.hasChanges;
          active.integrationStatus = captured.record.integrationStatus;
          if (integrationPending) {
            active.lifecycle = "awaiting_integration";
            active.worktree = await this.integrations.annotate(
              captured.record,
              this.port.worktreeRecoveryState(active, completed),
            );
            this.port.markPendingIntegration(
              agentId,
              active,
              completed,
              active.worktree,
            );
          }
        }
        completed = {
          ...completed,
          integration: this.port.worktreeSummary(active),
        };
        this.port.send({
          type: "subagent_integration",
          event: active.integrationStatus,
          agent: this.port.publicSubagent(active),
          integration: this.port.worktreeSummary(active),
          ...(active.worktree.integrationStatus === "conflicted"
            ? { error: "Patch conflicts with the current workspace" }
            : {}),
        });
      }
      if (
        options.resolutionForAgentId &&
        !integrationPending &&
        active.integrationStatus === "applied"
      ) {
        const source = this.port.resolveSource(options.resolutionForAgentId);
        if (source) {
          source.record = await this.integrations.resolvedBy(
            source.record,
            active.id,
          );
          source.agent.integrationStatus = "applied";
          this.port.deleteCompleted(options.resolutionForAgentId);
          this.port.send({
            type: "subagent_integration",
            event: "applied",
            agent: this.port.publicSubagent(source.agent),
            integration: this.port.worktreeSummary(source.agent),
            resolvedBy: active.id,
          });
        }
      }
      if (active.direct) {
        await this.port.injectDirectSubagentResult(active, completed);
      }
      this.port.finishSubagent(
        active,
        integrationPending ? "awaiting_integration" : "completed",
        completed,
      );
      return completed;
    } catch (error) {
      if (limitReached) {
        const limited: JsonObject = {
          status: "blocked",
          summary: `Subagent reached its ${profile.maxTurns}-turn limit`,
          evidence: [],
          changedPaths: [],
          verification: [],
          blockers: [`maxTurns ${profile.maxTurns} reached`],
          agentId,
          profile: options.profile,
          model: `${model.provider}/${model.id}`,
        };
        if (active.worktree && active.integrationStatus === "none") {
          try {
            const captured = await this.integrations.capture(active.worktree);
            active.worktree = captured.record;
            active.integrationStatus = captured.record.integrationStatus;
            if (captured.hasChanges) {
              active.lifecycle = "awaiting_integration";
              active.worktree = await this.integrations.annotate(
                captured.record,
                this.port.worktreeRecoveryState(active, limited),
              );
              this.port.markPendingIntegration(
                agentId,
                active,
                limited,
                active.worktree,
              );
              this.port.send({
                type: "subagent_integration",
                event: "pending",
                agent: this.port.publicSubagent(active),
                integration: this.port.worktreeSummary(active),
                error: String(limited.summary),
              });
            } else {
              await this.integrations.integrate(captured.record);
            }
          } catch (recoveryError) {
            this.port.reportHostWarning(
              `Unable to capture worktree changes for ${agentId} after its turn limit: ${
                recoveryError instanceof Error
                  ? recoveryError.message
                  : String(recoveryError)
              }. The registered checkout was preserved for recovery.`,
            );
          }
        }
        if (active.direct) {
          await this.port.injectDirectSubagentResult(active, limited);
        }
        this.port.finishSubagent(active, "limit_reached", limited);
        return limited;
      }
      const message = error instanceof Error ? error.message : String(error);
      if (options.resolutionForAgentId && active.worktree) {
        try {
          await this.integrations.discard(active.worktree);
          active.integrationStatus = "discarded";
        } catch (cleanupError) {
          this.port.reportHostWarning(
            `Unable to discard failed resolver worktree ${active.worktree.id}: ${
              cleanupError instanceof Error
                ? cleanupError.message
                : String(cleanupError)
            }`,
          );
        }
      } else if (active.worktree && active.integrationStatus === "none") {
        try {
          const captured = await this.integrations.capture(active.worktree);
          active.worktree = captured.record;
          active.integrationStatus = captured.record.integrationStatus;
          if (captured.hasChanges) {
            const failedResult: JsonObject = {
              status: "failed",
              summary: message,
              blockers: [message],
              integration: this.port.worktreeSummary(active),
            };
            active.lifecycle = "awaiting_integration";
            active.worktree = await this.integrations.annotate(
              captured.record,
              this.port.worktreeRecoveryState(active, failedResult),
            );
            this.port.markPendingIntegration(
              agentId,
              active,
              failedResult,
              active.worktree,
            );
            this.port.send({
              type: "subagent_integration",
              event: "pending",
              agent: this.port.publicSubagent(active),
              integration: this.port.worktreeSummary(active),
              error: message,
            });
          } else {
            await this.integrations.integrate(captured.record);
          }
        } catch (recoveryError) {
          this.port.reportHostWarning(
            `Unable to capture worktree changes for failed subagent ${agentId}: ${
              recoveryError instanceof Error
                ? recoveryError.message
                : String(recoveryError)
            }. The original execution error was preserved and the checkout remains registered.`,
          );
        }
      }
      this.port.finishSubagent(
        active,
        controller.signal.aborted ? "cancelled" : "failed",
        undefined,
        message,
      );
      throw error;
    } finally {
      unsubscribe();
      controller.signal.removeEventListener("abort", abortChild);
      this.port.removeActive(agentId);
      this.port.publishAgentsState();
    }
  }
}

function lastAssistantText(messages: unknown[]): string {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index];
    if (!isJsonObject(message) || message.role !== "assistant") continue;
    if (typeof message.content === "string") return message.content;
    if (!Array.isArray(message.content)) continue;
    const text = message.content
      .flatMap((block) =>
        isJsonObject(block) &&
        block.type === "text" &&
        typeof block.text === "string"
          ? [block.text]
          : [],
      )
      .join("\n");
    if (text) return text;
  }
  throw new Error("Subagent returned no assistant text");
}
