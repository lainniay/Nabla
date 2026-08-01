import { chmodSync, rmSync } from "node:fs";
import { homedir } from "node:os";
import { createServer, type Socket } from "node:net";
import { resolve } from "node:path";
import { AsyncLocalStorage } from "node:async_hooks";

import {
  type AgentSession,
  type AgentSessionRuntime,
  type CreateAgentSessionRuntimeFactory,
  DefaultResourceLoader,
  type InlineExtension,
  ModelRuntime,
  SessionManager,
  SettingsManager,
  copyToClipboard,
  createAgentSession,
  type ToolCallEvent,
  type ToolCallEventResult,
  createAgentSessionFromServices,
  createAgentSessionRuntime,
  createAgentSessionServices,
  getAgentDir,
  runRpcMode,
} from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";

import { ApprovalQueue, type ApprovalDecision } from "./approval.ts";
import { AuthPromptQueue } from "./auth-prompts.ts";
import {
  ContextBudgetManager,
  compactionRecordFromEntry,
  type ContextSnapshot,
} from "./context-manager.ts";
import {
  GoalStore,
  agentPermissionEffect,
  agentPermissionSummary,
  commandAllowedByLease,
  filterContextFilesByTrust,
  goalSpecFromToolParams,
  isCredentialPath,
  loadHarnessConfig,
  modelReference,
  pathAllowedByLease,
  saveWorkspaceTrust,
  workspaceIsTrusted,
  type AgentProfile,
  type GoalReview,
  type ReviewFinding,
  type GoalSpec,
  type HarnessConfig,
  type ResourceSnapshot,
  type TaskResult,
} from "./harness.ts";
import {
  isHighRiskCommand,
  hasShellControlSyntax,
  isSafeReadOnlyCommand,
  isManagedWorktreeCommand,
  MUTATING_TOOL_NAMES,
  READ_ONLY_TOOL_NAMES,
  THINKING_LEVELS,
} from "./policy/tool-policy.ts";
import { workspaceRelativePath } from "./policy/path-boundary.ts";
import {
  PLAN_ENTRY_TYPE,
  PLAN_EXECUTION_MESSAGE_TYPE,
  PLAN_MODE_ENTRY_TYPE,
  PlanStore,
  type PlanArtifactV2,
  type PlanContent,
  planExecutionPrompt,
  restorePlanMode,
} from "./plan.ts";
import {
  QuestionQueue,
  type PlanQuestion,
  type QuestionAnswer,
} from "./questions.ts";
import {
  SessionCatalog,
  buildTreeSnapshot,
  copyTextForEntry,
  createStartupSessionManager,
  projectSessionHistory,
  type TreeFilterMode,
} from "./session-navigation.ts";
import { workspacePathError } from "./workspace.ts";
import {
  parseSubagentOutput,
  type SubagentOutputKind,
} from "./protocol/subagent-output.ts";
import { CommandLanes } from "./protocol/command-lanes.ts";
import {
  isJsonObject,
  stringArray,
  type JsonObject,
} from "./protocol/validation.ts";
import type {
  ActiveAgentSnapshot,
  AgentsSnapshot,
  BootstrapState,
  WorktreeIntegrationSnapshot,
} from "./protocol/contracts.ts";
import {
  WorktreeManager,
  type IntegrationStatus,
  type IsolationBackend,
  type PreparedIsolation,
  type WorktreeRecoveryState,
  type WorktreeRecord,
} from "./worktree.ts";

type AuthType = Parameters<ModelRuntime["login"]>[1];
type AuthInteraction = Parameters<ModelRuntime["login"]>[2];
type AuthPrompt = Parameters<AuthInteraction["prompt"]>[0];
type AuthEvent = Parameters<AuthInteraction["notify"]>[0];

const PLAN_TOOLS = [
  ...READ_ONLY_TOOL_NAMES,
  "ask_user",
  "submit_plan",
  "delegate_task",
] as const;
const STANDARD_TOOLS = [
  ...READ_ONLY_TOOL_NAMES,
  "edit",
  "write",
  "bash",
  "delegate_task",
] as const;
const STANDARD_INSTRUCTIONS = [
  "Follow Pi's normal interactive agent behavior and the user's direct request.",
  "Do not create or advance a structured Goal unless the user explicitly invokes /goal.",
  "Mutation tools remain subject to the host's fine-grained approval policy.",
].join(" ");
const PLAN_INSTRUCTIONS = [
  "Nabla is in PLAN mode.",
  "Inspect the project and prepare a concrete implementation plan.",
  "Use ask_user only for ambiguities that materially change the implementation; record safe defaults as assumptions.",
  "A final plan MUST be submitted with submit_plan. Do not present ordinary assistant prose as the final plan.",
  "After submit_plan, stop and let the host present the review choices.",
  "Do not claim to have edited files or executed mutating commands.",
].join(" ");

interface ActiveFlow {
  id: string;
  controller: AbortController;
  prompts: AuthPromptQueue;
  nextPromptId: number;
}

interface SubagentOptions {
  task: string;
  profile: string;
  taskId?: string;
  goalId?: string;
  outputKind?: SubagentOutputKind;
  parentSignal?: AbortSignal;
  direct?: boolean;
  preparedIsolation?: PreparedIsolation;
  forceAutoIntegrate?: boolean;
  resolutionForAgentId?: string;
  discardWorktreeChanges?: boolean;
}

interface ActiveSubagent {
  id: string;
  profile: string;
  task: string;
  taskId?: string;
  goalId?: string;
  direct: boolean;
  planReadOnly: boolean;
  lifecycle:
    | "queued"
    | "preparing_isolation"
    | "running"
    | "awaiting_integration"
    | "resolving";
  session?: AgentSession;
  originSession: AgentSession;
  originSessionId: string;
  controller: AbortController;
  startedAt: string;
  turns: number;
  maxTurns: number;
  model: string;
  isolationBackend: IsolationBackend;
  integrationStatus: IntegrationStatus;
  isolationWarning?: string;
  worktree?: WorktreeRecord;
}

interface SubagentHandle {
  agent: ActiveSubagent;
  completion: Promise<JsonObject>;
}

class PlanModeController {
  private active = false;
  private runtime?: AgentSessionRuntime;

  current(): boolean {
    return this.active;
  }

  attach(runtime: AgentSessionRuntime): void {
    this.runtime = runtime;
    this.applyToSession(runtime.session, this.active);
  }

  runtimeHandle(): AgentSessionRuntime {
    if (!this.runtime) throw new Error("Agent runtime is not ready");
    return this.runtime;
  }

  apply(session: AgentSession): void {
    this.applyToSession(session, this.active);
  }

  restore(session: AgentSession, active: boolean): readonly string[] {
    const activeTools = this.applyToSession(session, active);
    this.active = active;
    return activeTools;
  }

  set(active: boolean): readonly string[] {
    const runtime = this.runtime;
    if (!runtime) throw new Error("Agent runtime is not ready");
    if (!runtime.session.isIdle) {
      throw new Error("Cannot switch mode while the agent is running");
    }
    const activeTools = this.applyToSession(runtime.session, active);
    this.active = active;
    return activeTools;
  }

  private applyToSession(session: AgentSession, active: boolean): string[] {
    const expected = [...toolsForPlanMode(active)];
    const previous = session.getActiveToolNames();
    session.setActiveToolsByName(expected);
    const activeTools = session.getActiveToolNames();
    const missing = expected.filter((tool) => !activeTools.includes(tool));
    if (missing.length > 0) {
      session.setActiveToolsByName(previous);
      throw new Error(`Pi did not register required tools: ${missing.join(", ")}`);
    }
    return activeTools;
  }
}

class HostBridge {
  private socket?: Socket;
  private activeFlow?: ActiveFlow;
  private readonly approvals = new ApprovalQueue();
  private readonly questions = new QuestionQueue();
  private readonly plans: PlanStore;
  private readonly server;
  private readonly socketPath: string;
  private readonly modelRuntime: ModelRuntime;
  private readonly planMode: PlanModeController;
  private readonly contextBudget: ContextBudgetManager;
  private readonly goals: GoalStore;
  private readonly afterLogin: (providerId: string) => Promise<unknown>;
  private readonly sessionCatalogs = new Map<string, SessionCatalog>();
  private readonly subagents = new Map<string, ActiveSubagent>();
  private readonly completedSubagents = new Map<
    string,
    { agent: ActiveSubagent; result: JsonObject; record: WorktreeRecord }
  >();
  private readonly worktrees = new WorktreeManager({
    credentialPath: isCredentialPath,
  });
  private config: HarnessConfig;
  private resourceRevision = 1;
  private agentsRevision = 0;
  private subagentSequence = 0;
  private goalOperationGeneration = 0;
  private goalPreparationRunning = false;
  private goalAutomationRunning = false;
  private writeSubagentTail: Promise<unknown> = Promise.resolve();
  private replacementPlan?: PlanArtifactV2;
  private readonly worktreeRecoveryWarnings: string[] = [];
  private readonly commandLanes = new CommandLanes();
  private readonly requestSockets = new Map<string, Socket>();
  private readonly requestContext = new AsyncLocalStorage<{
    id?: string;
    socket: Socket;
  }>();
  private readonly capacityWaiters = new Set<() => void>();
  private connectionGeneration = 0;

  constructor(
    socketPath: string,
    modelRuntime: ModelRuntime,
    planMode: PlanModeController,
    plans: PlanStore,
    contextBudget: ContextBudgetManager,
    goals: GoalStore,
    config: HarnessConfig,
    afterLogin: (providerId: string) => Promise<unknown>,
  ) {
    this.socketPath = socketPath;
    this.modelRuntime = modelRuntime;
    this.planMode = planMode;
    this.plans = plans;
    this.contextBudget = contextBudget;
    this.goals = goals;
    this.config = config;
    this.afterLogin = afterLogin;
    this.server = createServer((socket) => this.accept(socket));
  }

  extension(): InlineExtension {
    return {
      name: "nabla-control",
      factory: (pi) => {
        pi.registerTool({
          name: "ask_user",
          label: "Ask user",
          description:
            "Ask the user 1-3 material clarification questions. Each question is single-select and always allows a custom answer in the host UI.",
          promptSnippet: "Ask structured clarification questions when a material product decision is missing",
          parameters: Type.Object({
            questions: Type.Array(
              Type.Object({
                id: Type.String({ minLength: 1 }),
                prompt: Type.String({ minLength: 1 }),
                options: Type.Array(
                  Type.Object({
                    id: Type.String({ minLength: 1 }),
                    label: Type.String({ minLength: 1 }),
                    description: Type.Optional(Type.String()),
                  }),
                  { minItems: 2, maxItems: 4 },
                ),
              }),
              { minItems: 1, maxItems: 3 },
            ),
          }),
          execute: async (_toolCallId, params, signal) => {
            const questions = params.questions as PlanQuestion[];
            const answers = await this.questions.request(
              questions,
              signal,
              (requestId, requestedQuestions) =>
                this.send({
                  type: "question_request",
                  requestId,
                  questions: requestedQuestions,
                }),
              (requestId) => this.send({ type: "question_cancelled", requestId }),
            );
            return {
              content: [{ type: "text", text: JSON.stringify({ answers }) }],
              details: { answers },
            };
          },
        });
        pi.registerTool({
          name: "submit_plan",
          label: "Submit plan",
          description:
            "Submit the final implementation plan as a structured artifact for user review. This terminates the current planning turn.",
          promptSnippet: "Submit the final implementation plan artifact",
          parameters: Type.Object({
            title: Type.String({ minLength: 1 }),
            summary: Type.String({ minLength: 1 }),
            bodyMarkdown: Type.String({ minLength: 1 }),
            assumptions: Type.Array(Type.String()),
            testPlan: Type.Array(Type.String()),
          }),
          execute: async (_toolCallId, params, _signal, _onUpdate, context) => {
            if (!this.planMode.current()) {
              throw new Error("submit_plan is only available in Plan mode");
            }
            const artifact = this.plans.submit(
              params as PlanContent,
              context.sessionManager.getSessionId(),
            );
            pi.appendEntry(PLAN_ENTRY_TYPE, artifact);
            this.send({ type: "plan_ready", artifact });
            return {
              content: [
                {
                  type: "text",
                  text: `Plan ${artifact.id} revision ${artifact.revision} was submitted for review.`,
                },
              ],
              details: { artifact },
              terminate: true,
            };
          },
        });
        pi.registerTool({
          name: "delegate_task",
          label: "Delegate task",
          description:
            "Run a bounded task in an independent in-process agent session using a configured planner, worker, verifier, or reviewer profile.",
          promptSnippet:
            "Delegate independent bounded work to a configured subagent profile",
          parameters: Type.Object({
            task: Type.String({ minLength: 1 }),
            profile: Type.Optional(Type.String()),
            taskId: Type.Optional(Type.String()),
          }),
          execute: async (_toolCallId, params, signal) => {
            const profile =
              params.profile ??
              (this.planMode.current() ? "planner" : "worker");
            const result = await this.runSubagent({
              task: params.task,
              profile,
              taskId: params.taskId,
              goalId: params.taskId ? this.goals.active()?.id : undefined,
              parentSignal: signal,
            });
            return {
              content: [{ type: "text", text: JSON.stringify(result) }],
              details: result,
            };
          },
        });
        pi.on("session_start", (_event, context) => {
          this.contextBudget.onSessionStart(
            context.sessionManager.getSessionId(),
          );
          const goalSnapshot = this.goals.attach(
            context.cwd,
            context.sessionManager.getSessionId(),
          );
          this.send({ type: "goal_state", snapshot: goalSnapshot });
          this.sendContextBudget(
            this.contextBudget.onModelResponse(context.getContextUsage()),
          );
          if (
            this.replacementPlan &&
            !context.sessionManager
              .getEntries()
              .some(
                (entry) => entry.type === "custom" && entry.customType === PLAN_ENTRY_TYPE,
              )
          ) {
            this.plans.adopt(this.replacementPlan);
            this.send({ type: "plan_state", artifact: this.replacementPlan });
            return;
          }
          const restored = this.plans.restore(context.sessionManager.getBranch());
          if (restored.recovered && restored.artifact) {
            pi.appendEntry(PLAN_ENTRY_TYPE, restored.artifact);
          }
          this.send({ type: "plan_state", artifact: restored.artifact ?? null });
        });
        pi.on("before_agent_start", (event) => {
          return {
            systemPrompt: [
              event.systemPrompt,
              this.planMode.current()
                ? PLAN_INSTRUCTIONS
                : STANDARD_INSTRUCTIONS,
              this.subagentCatalogPrompt(),
            ]
              .filter(Boolean)
              .join("\n\n"),
          };
        });
        pi.on("context", (event, context) => {
          const result = this.contextBudget.filter(
            event.messages,
            context.getContextUsage(),
            {
              planMode: this.planMode.current(),
              plan: this.plans.latest(),
              goal: this.goals.goalView(),
            },
          );
          this.sendContextBudget(result.snapshot);
          return { messages: result.messages };
        });
        pi.on("turn_end", (_event, context) => {
          this.sendContextBudget(
            this.contextBudget.onModelResponse(context.getContextUsage()),
          );
        });
        pi.on("session_compact", (event) => {
          this.sendContextBudget(
            this.contextBudget.onCompaction(
              compactionRecordFromEntry(event.reason, event.compactionEntry),
            ),
          );
        });
        pi.on("tool_call", (event, context) =>
          this.approveTool(event, context.cwd, context.signal),
        );
        pi.on("agent_settled", () => {
          this.completePlanExecution();
        });
      },
    };
  }

  async listen(): Promise<void> {
    rmSync(this.socketPath, { force: true });
    await this.recoverWorktrees();
    await new Promise<void>((resolve, reject) => {
      this.server.once("error", reject);
      this.server.listen(this.socketPath, () => {
        this.server.off("error", reject);
        chmodSync(this.socketPath, 0o600);
        resolve();
      });
    });
  }

  async close(): Promise<void> {
    this.cancelActiveFlow("Authentication host stopped");
    this.approvals.denyAll();
    this.questions.cancelAll();
    const activeSubagents = [...this.subagents.values()];
    for (const subagent of activeSubagents) subagent.controller.abort();
    await Promise.allSettled(
      activeSubagents.flatMap((subagent) =>
        subagent.session ? [subagent.session.abort()] : [],
      ),
    );
    this.socket?.destroy();
    this.requestSockets.clear();
    await new Promise<void>((resolve) => this.server.close(() => resolve()));
    rmSync(this.socketPath, { force: true });
  }

  private accept(socket: Socket): void {
    const generation = ++this.connectionGeneration;
    if (this.socket) {
      this.cancelActiveFlow("Host control client replaced");
      this.approvals.denyAll();
      this.questions.cancelAll("Host control client replaced");
      this.forgetSocketRequests(this.socket);
    }
    this.socket?.destroy();
    this.socket = socket;
    socket.setEncoding("utf8");

    let buffered = "";
    socket.on("data", (chunk: string) => {
      buffered += chunk;
      while (true) {
        const newline = buffered.indexOf("\n");
        if (newline < 0) break;
        const line = buffered.slice(0, newline).replace(/\r$/u, "");
        buffered = buffered.slice(newline + 1);
        if (line.length > 0) this.dispatchLine(line, socket, generation);
      }
    });
    socket.on("close", () => {
      this.forgetSocketRequests(socket);
      if (this.socket !== socket) return;
      this.connectionGeneration += 1;
      this.socket = undefined;
      this.cancelActiveFlow("Authentication client disconnected");
      this.approvals.denyAll();
      this.questions.cancelAll("Host control client disconnected");
    });
  }

  private send(message: JsonObject): void {
    if (!this.socket || this.socket.destroyed) return;
    const scopeId = this.tryCurrentScopeId();
    const scoped =
      scopeId && typeof message.scopeId !== "string"
        ? { ...message, scopeId }
        : message;
    this.socket.write(`${JSON.stringify(scoped)}\n`);
  }

  private sendTo(socket: Socket, message: JsonObject): void {
    if (socket.destroyed) return;
    socket.write(`${JSON.stringify(message)}\n`);
  }

  private forgetSocketRequests(socket: Socket): void {
    for (const [id, target] of this.requestSockets) {
      if (target === socket) this.requestSockets.delete(id);
    }
  }

  private sendContextBudget(snapshot: ContextSnapshot): void {
    if (!this.socket || this.socket.destroyed) return;
    const policyWarning = this.contextBudget.takeWarning();
    this.send({
      type: "context_budget",
      snapshot: this.contextSnapshot(snapshot),
      ...(policyWarning ? { policyWarning } : {}),
    });
  }

  private response(
    id: string | undefined,
    command: string,
    success: boolean,
    data?: unknown,
    error?: string,
  ): void {
    const context = this.requestContext.getStore();
    const target =
      context && context.id === id
        ? context.socket
        : id
          ? this.requestSockets.get(id)
          : this.socket;
    if (id) this.requestSockets.delete(id);
    if (!target) return;
    this.sendTo(target, {
      id,
      type: "response",
      command,
      success,
      ...(data === undefined ? {} : { data }),
      ...(error === undefined ? {} : { error }),
    });
  }

  private dispatchLine(line: string, socket: Socket, generation: number): void {
    let request: JsonObject;
    try {
      request = JSON.parse(line) as JsonObject;
    } catch (error) {
      this.sendTo(socket, {
        type: "host_protocol_error",
        error: error instanceof Error ? error.message : String(error),
      });
      return;
    }
    const id = typeof request.id === "string" ? request.id : undefined;
    if (id) this.requestSockets.set(id, socket);
    const lane = commandLane(request);
    void this.commandLanes
      .run(lane, async () => {
        if (generation !== this.connectionGeneration || socket.destroyed) {
          if (id) this.requestSockets.delete(id);
          return;
        }
        await this.requestContext.run({ id, socket }, () =>
          this.handleRequest(request),
        );
      })
      .catch((error) => {
        if (id) this.requestSockets.delete(id);
        this.sendTo(socket, {
          type: "host_protocol_error",
          error: error instanceof Error ? error.message : String(error),
        });
      });
  }

  private async handleRequest(request: JsonObject): Promise<void> {
    const id = typeof request.id === "string" ? request.id : undefined;
    const command = typeof request.type === "string" ? request.type : "";
    try {
      switch (command) {
        case "auth_list":
          this.response(id, command, true, {
            providers: await this.listProviders(),
          });
          break;
        case "bootstrap_state":
          this.response(id, command, true, this.bootstrapState());
          break;
        case "auth_login":
          this.startLogin(id, request);
          break;
        case "auth_reply":
          this.replyToPrompt(id, request);
          break;
        case "auth_cancel":
          this.cancelActiveFlow("Login cancelled");
          this.response(id, command, true);
          break;
        case "auth_logout":
          await this.logout(id, request);
          break;
        case "set_plan_mode":
          this.setPlanMode(id, request);
          break;
        case "question_reply":
          this.replyQuestion(id, request);
          break;
        case "get_plan_state":
          this.response(id, command, true, {
            scopeId: this.currentScopeId(),
            artifact: this.plans.latest(),
          });
          break;
        case "context_state":
          this.response(id, command, true, this.contextSnapshot());
          break;
        case "resource_state":
          this.response(id, command, true, this.resourceSnapshot());
          break;
        case "resource_reload":
          await this.reloadResources(id);
          break;
        case "workspace_trust":
          await this.setWorkspaceTrust(id, request);
          break;
        case "goal_state":
          this.response(id, command, true, this.goalSnapshot());
          break;
        case "goals_state":
          this.response(id, command, true, this.goals.list());
          break;
        case "goal_start":
          this.startGoal(id, request);
          break;
        case "goal_action":
          this.goalAction(id, request);
          break;
        case "goal_approve":
          this.approveGoal(id);
          break;
        case "queue_clear":
          this.clearQueue(id);
          break;
        case "model_list":
          await this.listModels(id);
          break;
        case "model_set":
          await this.setModel(id, request);
          break;
        case "thinking_set":
          this.setThinking(id, request);
          break;
        case "agents_state":
          this.response(id, command, true, this.agentsSnapshot());
          break;
        case "agents_reload":
          await this.reloadAgents(id);
          break;
        case "subagent_start":
          this.startDirectSubagent(id, request);
          break;
        case "subagent_cancel":
          await this.cancelSubagent(id, request);
          break;
        case "subagent_integrate":
          await this.integrateSubagent(id, request);
          break;
        case "session_browser_open":
          await this.openSessionBrowser(id);
          break;
        case "session_browser_query":
          await this.querySessionBrowser(id, request);
          break;
        case "session_browser_close":
          this.closeSessionBrowser(id, request);
          break;
        case "session_new":
          await this.newSession(id);
          break;
        case "session_resume":
          await this.resumeSession(id, request);
          break;
        case "tree_state":
          this.treeState(id, request);
          break;
        case "tree_label":
          this.setTreeLabel(id, request);
          break;
        case "tree_copy":
          await this.copyTreeEntry(id, request);
          break;
        case "tree_navigate":
          await this.navigateTree(id, request);
          break;
        case "tree_abort":
          this.abortTreeNavigation(id);
          break;
        case "execute_plan_current":
          await this.executePlan(id, false);
          break;
        case "execute_plan_fresh":
          await this.executePlan(id, true);
          break;
        case "approval_reply":
          this.replyApproval(id, request);
          break;
        default:
          this.response(id, command || "unknown", false, undefined, "Unknown host command");
      }
    } catch (error) {
      this.response(
        id,
        command || "unknown",
        false,
        undefined,
        error instanceof Error ? error.message : String(error),
      );
    }
  }

  private currentScopeId(): string {
    return this.planMode.runtimeHandle().session.sessionId;
  }

  private tryCurrentScopeId(): string | undefined {
    try {
      return this.currentScopeId();
    } catch {
      return undefined;
    }
  }

  private contextSnapshot(
    snapshot = this.contextBudget.snapshot(),
  ): ContextSnapshot {
    return { ...snapshot, scopeId: this.currentScopeId() };
  }

  private goalSnapshot(goal = this.goals.current()): ReturnType<GoalStore["snapshot"]> {
    return {
      ...this.goals.snapshot(),
      scopeId: this.currentScopeId(),
      goal: goal ?? null,
    };
  }

  private hasMutableGoalTask(taskId: string, goalId?: string): boolean {
    const goal = this.goals.active();
    return (
      goal !== undefined &&
      goal.stage !== "blocked" &&
      (!goalId || goal.id === goalId) &&
      goal.tasks.some((task) => task.id === taskId)
    );
  }

  private reportHostWarning(message: string): void {
    if (!this.worktreeRecoveryWarnings.includes(message)) {
      this.worktreeRecoveryWarnings.push(message);
    }
    this.send({ type: "host_warning", message });
  }

  private resourceSnapshot(
    session = this.planMode.runtimeHandle().session,
  ): ResourceSnapshot {
    const loader = session.resourceLoader;
    const skills = loader.getSkills();
    const prompts = loader.getPrompts();
    const extensions = loader.getExtensions();
    return {
      scopeId: session.sessionId,
      trusted: workspaceIsTrusted(
        session.sessionManager.getCwd(),
        this.config,
      ),
      contextFiles: loader.getAgentsFiles().agentsFiles.map((file) => file.path),
      skills: skills.skills.map((skill) => ({
        name: skill.name,
        path: skill.filePath,
        description: skill.description,
      })),
      prompts: prompts.prompts.map((prompt) => ({
        name: prompt.name,
        path: prompt.filePath,
        description: prompt.description,
      })),
      extensions: extensions.extensions.map(
        (extension) => extension.resolvedPath,
      ),
      commands: [
        ...extensions.extensions.flatMap((extension) =>
          [...extension.commands.values()].map((command) => ({
            name: command.name,
            description: command.description ?? "",
            source: "extension" as const,
          })),
        ),
        ...prompts.prompts.map((prompt) => ({
          name: prompt.name,
          description: prompt.description,
          source: "prompt" as const,
        })),
        ...skills.skills.map((skill) => ({
          name: `skill:${skill.name}`,
          description: skill.description,
          source: "skill" as const,
        })),
      ],
      diagnostics: [
        ...skills.diagnostics,
        ...prompts.diagnostics,
        ...extensions.errors.map((error) => ({
          type: "error",
          message: error.error,
          path: error.path,
        })),
      ],
      revision: this.resourceRevision,
    };
  }

  private bootstrapState(): BootstrapState {
    const session = this.planMode.runtimeHandle().session;
    return {
      scopeId: session.sessionId,
      planMode: {
        active: this.planMode.current(),
        activeTools: session.getActiveToolNames(),
      },
      plan: { artifact: this.plans.latest() ?? null },
      resources: this.resourceSnapshot(),
      goal: this.goalSnapshot(),
      agents: this.agentsSnapshot(session),
      context: this.contextSnapshot(),
      pendingIntegrations: [...this.completedSubagents.values()].map(
        ({ agent }) => ({
          agent: this.publicSubagent(agent),
          integration: this.worktreeSummary(agent),
        }),
      ),
      warnings: [...this.worktreeRecoveryWarnings],
    };
  }

  private publishWorkspaceState(
    session = this.planMode.runtimeHandle().session,
  ): { resources: ResourceSnapshot; agents: AgentsSnapshot } {
    this.resourceRevision += 1;
    this.agentsRevision += 1;
    const resources = this.resourceSnapshot(session);
    const agents = this.agentsSnapshot(session);
    this.send({
      type: "workspace_state",
      scopeId: session.sessionId,
      resources,
      agents,
    });
    return { resources, agents };
  }

  private async reloadResources(id: string | undefined): Promise<void> {
    const runtime = this.planMode.runtimeHandle();
    if (!runtime.session.isIdle) {
      throw new Error("Cannot reload resources while the agent is running");
    }
    this.config = loadHarnessConfig(
      runtime.session.sessionManager.getCwd(),
    );
    await runtime.session.reload();
    this.planMode.apply(runtime.session);
    this.sendPlanModeState();
    const { resources } = this.publishWorkspaceState(runtime.session);
    this.response(id, "resource_reload", true, resources);
  }

  activateWorkspace(cwd: string, session?: AgentSession): void {
    this.config = loadHarnessConfig(cwd);
    if (session && this.socket && !this.socket.destroyed) {
      this.publishWorkspaceState(session);
    }
  }

  private async setWorkspaceTrust(
    id: string | undefined,
    request: JsonObject,
  ): Promise<void> {
    const runtime = this.planMode.runtimeHandle();
    if (!runtime.session.isIdle) {
      throw new Error("Cannot change workspace trust while the agent is running");
    }
    const trusted = request.trusted === true;
    const cwd = runtime.session.sessionManager.getCwd();
    this.config = saveWorkspaceTrust(cwd, trusted);
    this.config = loadHarnessConfig(cwd);
    runtime.services.settingsManager.setProjectTrusted(trusted);
    await runtime.session.resourceLoader.reload({
      resolveProjectTrust: async () => trusted,
    });
    await runtime.session.reload();
    this.planMode.apply(runtime.session);
    this.sendPlanModeState();
    const { resources } = this.publishWorkspaceState(runtime.session);
    this.response(id, "workspace_trust", true, resources);
  }

  private sendGoalState(goal = this.goals.current()): void {
    this.send({
      type: "goal_state",
      snapshot: this.goalSnapshot(goal),
    });
  }

  private startGoal(id: string | undefined, request: JsonObject): void {
    const constraints = stringArrayField(request, "constraints");
    const fromPlan = request.fromPlan === true;
    const sourcePlan = fromPlan ? this.plans.latest() : undefined;
    if (fromPlan && !sourcePlan) {
      throw new Error("No submitted Plan is available on the current branch");
    }
    if (sourcePlan?.status === "executing") {
      throw new Error("Cannot create a Goal from a Plan that is executing");
    }
    const objective =
      optionalStringField(request, "objective")?.trim() ||
      sourcePlan?.title ||
      "";
    if (!objective) throw new Error("Goal objective must not be empty");
    const goal = this.goals.start(objective, constraints, sourcePlan);
    this.goalOperationGeneration += 1;
    this.sendGoalState(goal);
    this.response(id, "goal_start", true, this.goalSnapshot());
    void this.prepareGoal(goal.id);
  }

  private goalAction(id: string | undefined, request: JsonObject): void {
    const action = enumField(
      request,
      "action",
      ["pause", "resume", "cancel"] as const,
    );
    let goal;
    if (action === "pause") {
      if (this.goals.current()?.stage === "paused") {
        throw new Error("Goal is already paused");
      }
      this.goalOperationGeneration += 1;
      this.cancelGoalSubagents(this.goals.current()?.id);
      goal = this.goals.transition("paused");
    } else if (action === "resume") {
      this.goalOperationGeneration += 1;
      goal = this.goals.resume();
      if (goal.stage === "preparing") {
        void this.prepareGoal(goal.id);
      } else if (goal.stage === "awaiting_approval" && goal.spec) {
        this.send({ type: "goal_spec_ready", snapshot: this.goalSnapshot() });
      } else if (goal.stage === "executing") {
        void this.runGoalExecution();
      } else if (goal.stage === "verifying" || goal.stage === "reviewing") {
        goal = this.goals.transition("executing");
        void this.runGoalExecution();
      }
    } else {
      this.goalOperationGeneration += 1;
      this.cancelGoalSubagents(this.goals.current()?.id);
      goal = this.goals.transition("cancelled");
    }
    this.sendGoalState(goal);
    this.response(id, "goal_action", true, this.goalSnapshot());
  }

  private approveGoal(id: string | undefined): void {
    const goal = this.goals.approveSpec();
    this.sendGoalState(goal);
    this.response(id, "goal_approve", true, this.goalSnapshot());
    void this.runGoalExecution();
  }

  private async prepareGoal(goalId: string): Promise<void> {
    if (this.goalPreparationRunning) return;
    const generation = this.goalOperationGeneration;
    this.goalPreparationRunning = true;
    try {
      const goal = this.goals.active();
      if (!goal || goal.id !== goalId || goal.stage !== "preparing") return;
      const result = await this.runSubagent({
        profile: "planner",
        goalId,
        outputKind: "goal_spec",
        task: [
          `Prepare an executable Goal specification for: ${goal.objective}`,
          goal.constraints.length > 0
            ? `Constraints:\n${goal.constraints.map((item) => `- ${item}`).join("\n")}`
            : "",
          goal.sourcePlan
            ? `Source Plan snapshot:\n${planExecutionPrompt(goal.sourcePlan.artifact)}`
            : "",
          "Inspect the workspace without modifying it.",
          "Return summary, acceptanceCriteria, allowedTools, allowedPaths, allowedCommands, and dependency-aware tasks.",
        ]
          .filter(Boolean)
          .join("\n\n"),
      });
      const current = this.goals.active();
      if (!current || current.id !== goalId || current.stage !== "preparing") {
        return;
      }
      const spec = goalSpecFromToolParams(result, {
        fallbackSummary: current.objective,
        sourcePlan: current.sourcePlan?.artifact,
      });
      this.validateGoalSpecProfiles(spec);
      const prepared = this.goals.acceptSpec(spec);
      this.sendGoalState(prepared);
      this.send({ type: "goal_spec_ready", snapshot: this.goalSnapshot() });
    } catch (error) {
      if (generation === this.goalOperationGeneration) {
        this.goalFailed(
          `Goal preparation failed: ${
            error instanceof Error ? error.message : String(error)
          }`,
        );
      }
    } finally {
      this.goalPreparationRunning = false;
      const current = this.goals.active();
      if (
        generation !== this.goalOperationGeneration &&
        current?.id === goalId &&
        current.stage === "preparing"
      ) {
        void this.prepareGoal(goalId);
      }
    }
  }

  private validateGoalSpecProfiles(spec: Omit<GoalSpec, "revision">): void {
    const requiredProfiles = [
      ...spec.tasks.map((task) => task.profile ?? "worker"),
      "verifier",
      "reviewer",
    ];
    const unknown = requiredProfiles.filter(
      (profile) => !this.config.profiles[profile],
    );
    if (unknown.length > 0) {
      throw new Error(
        `Goal uses unknown subagent profiles: ${[...new Set(unknown)].join(", ")}`,
      );
    }
    const unavailable = [...new Set(requiredProfiles)].flatMap((name) => {
      const profile = this.config.profiles[name];
      if (!profile) return [];
      if (profile.disabled) return [`${name}: disabled`];
      const reason = this.profileUnavailableReason(profile);
      return reason ? [`${name}: ${reason}`] : [];
    });
    if (unavailable.length > 0) {
      throw new Error(
        `Goal uses unavailable subagent profiles: ${unavailable.join("; ")}`,
      );
    }
  }

  private cancelGoalSubagents(goalId: string | undefined): void {
    if (!goalId) return;
    for (const subagent of this.subagents.values()) {
      if (subagent.goalId !== goalId) continue;
      subagent.controller.abort();
      if (subagent.session) void subagent.session.abort();
    }
    this.notifySubagentCapacity();
  }

  private clearQueue(id: string | undefined): void {
    const queue = this.planMode.runtimeHandle().session.clearQueue();
    this.response(id, "queue_clear", true, {
      ...queue,
      restoredText: [...queue.steering, ...queue.followUp].join("\n\n"),
    });
  }

  private async listModels(id: string | undefined): Promise<void> {
    const runtime = this.planMode.runtimeHandle();
    const models = await this.modelRuntime.getAvailable();
    this.response(id, "model_list", true, {
      current: runtime.session.model
        ? {
            provider: runtime.session.model.provider,
            id: runtime.session.model.id,
          }
        : null,
      models: models.map((model) => ({
        provider: model.provider,
        id: model.id,
        name: model.name,
        reasoning: model.reasoning,
        contextWindow: model.contextWindow,
      })),
    });
  }

  private async setModel(
    id: string | undefined,
    request: JsonObject,
  ): Promise<void> {
    const runtime = this.planMode.runtimeHandle();
    if (!runtime.session.isIdle) {
      throw new Error("Cannot change model while the agent is running");
    }
    const provider = stringField(request, "provider");
    const modelId = stringField(request, "modelId");
    const model = this.modelRuntime.getModel(provider, modelId);
    if (!model) throw new Error(`Unknown model: ${provider}/${modelId}`);
    await runtime.session.setModel(model);
    this.response(id, "model_set", true, {
      provider,
      id: modelId,
      name: model.name,
    });
  }

  private setThinking(id: string | undefined, request: JsonObject): void {
    const runtime = this.planMode.runtimeHandle();
    if (!runtime.session.isIdle) {
      throw new Error("Cannot change thinking level while the agent is running");
    }
    const level = enumField(request, "level", THINKING_LEVELS);
    runtime.session.setThinkingLevel(level);
    this.response(id, "thinking_set", true, {
      level: runtime.session.thinkingLevel,
      available: runtime.session.getAvailableThinkingLevels(),
    });
  }

  private agentsSnapshot(
    session = this.planMode.runtimeHandle().session,
  ): AgentsSnapshot {
    return {
      scopeId: session.sessionId,
      revision: this.agentsRevision,
      maxParallel: this.config.maxParallel,
      profiles: Object.entries(this.config.profiles).map(([name, profile]) => ({
        unavailableReason:
          this.profileUnavailableReason(profile, session) ?? null,
        name,
        description: profile.description,
        source: profile.source,
        model: profile.model ?? null,
        thinkingLevel: profile.thinkingLevel ?? null,
        skills: profile.skills,
        tools: profile.tools,
        permission: agentPermissionSummary(profile),
        maxParallel: profile.maxParallel,
        maxTurns: profile.maxTurns,
        isolation: profile.isolation,
        disabled: profile.disabled,
      })),
      active: [...this.subagents.values()].map((agent) =>
        this.publicSubagent(agent),
      ),
      pending: [...this.completedSubagents.values()].map(({ agent }) =>
        this.publicSubagent(agent),
      ),
      diagnostics: this.config.diagnostics,
    };
  }

  private publishAgentsState(
    session = this.planMode.runtimeHandle().session,
  ): AgentsSnapshot {
    this.agentsRevision += 1;
    const snapshot = this.agentsSnapshot(session);
    this.send({ type: "agents_state", snapshot });
    return snapshot;
  }

  private async recoverWorktrees(): Promise<void> {
    const runtime = this.planMode.runtimeHandle();
    const cwd = runtime.session.sessionManager.getCwd();
    const recovery = await this.worktrees.listRecoverable(cwd);
    this.worktreeRecoveryWarnings.push(...recovery.warnings);
    const records = recovery.records;
    for (let record of records) {
      const metadata = record.recovery;
      if (!this.validWorktreeRecovery(metadata)) {
        this.worktreeRecoveryWarnings.push(
          `Preserved worktree ${record.id}, but its recovery metadata is missing or invalid.`,
        );
        continue;
      }
      const profile = this.config.profiles[metadata.profile];
      if (!profile) {
        this.worktreeRecoveryWarnings.push(
          `Preserved worktree ${record.id}, but subagent profile ${metadata.profile} is unavailable.`,
        );
        continue;
      }
      try {
        if (record.integrationStatus === "none") {
          const captured = await this.worktrees.capture(record);
          record = captured.record;
          if (!captured.hasChanges) {
            await this.worktrees.integrate(record);
            continue;
          }
        }
        this.validateWorktreePaths(
          record,
          profile,
          record.originWorkspace,
          metadata.taskId,
          metadata.goalId,
        );
      } catch (error) {
        let warning =
          `Preserved worktree ${record.id}, but recovery validation failed: ${
            error instanceof Error ? error.message : String(error)
          }`;
        try {
          await this.worktrees.keep(record);
        } catch (keepError) {
          warning += `; recording the keep decision also failed: ${
            keepError instanceof Error ? keepError.message : String(keepError)
          }`;
        }
        this.worktreeRecoveryWarnings.push(warning);
        continue;
      }
      const sequence = /^agent-(\d+)$/u.exec(record.agentId)?.[1];
      if (sequence) {
        this.subagentSequence = Math.max(
          this.subagentSequence,
          Number.parseInt(sequence, 10),
        );
      }
      const result =
        metadata.result && isJsonObject(metadata.result)
          ? metadata.result
          : {
              status: "blocked",
              summary:
                "Recovered isolated subagent changes after the host restarted",
              evidence: [],
              changedPaths: record.changedPaths,
              verification: [],
              blockers: ["Integration was interrupted before completion"],
            };
      const active: ActiveSubagent = {
        id: record.agentId,
        profile: metadata.profile,
        task: metadata.task,
        taskId: metadata.taskId,
        goalId: metadata.goalId,
        direct: metadata.direct,
        planReadOnly: metadata.planReadOnly,
        lifecycle: "awaiting_integration",
        originSession: runtime.session,
        originSessionId: metadata.originSessionId,
        controller: new AbortController(),
        startedAt: record.createdAt,
        turns: 0,
        maxTurns: profile.maxTurns,
        model: metadata.model,
        isolationBackend: "worktree",
        integrationStatus: record.integrationStatus,
        worktree: record,
      };
      this.completedSubagents.set(active.id, {
        agent: active,
        result,
        record,
      });
    }
    await this.worktrees.pruneTerminalArtifacts(cwd).catch((error) => {
      this.worktreeRecoveryWarnings.push(
        `Unable to prune old terminal worktree artifacts: ${
          error instanceof Error ? error.message : String(error)
        }`,
      );
    });
  }

  private validWorktreeRecovery(
    value: WorktreeRecoveryState | undefined,
  ): value is WorktreeRecoveryState {
    return (
      value !== undefined &&
      typeof value.profile === "string" &&
      typeof value.task === "string" &&
      typeof value.direct === "boolean" &&
      typeof value.planReadOnly === "boolean" &&
      typeof value.model === "string" &&
      typeof value.originSessionId === "string" &&
      (value.taskId === undefined || typeof value.taskId === "string") &&
      (value.goalId === undefined || typeof value.goalId === "string") &&
      (value.result === undefined || isJsonObject(value.result))
    );
  }

  private profileUnavailableReason(
    profile: AgentProfile,
    session = this.planMode.runtimeHandle().session,
  ): string | undefined {
    const availableSkills = new Set(
      session.resourceLoader.getSkills().skills.map((skill) => skill.name),
    );
    const missingSkills = profile.skills.filter(
      (skill) => !availableSkills.has(skill),
    );
    if (missingSkills.length > 0) {
      return `Missing skills: ${missingSkills.join(", ")}`;
    }
    const reference = modelReference(profile);
    if (
      reference &&
      !this.modelRuntime.getModel(reference.provider, reference.id)
    ) {
      return `Configured model is unavailable: ${reference.provider}/${reference.id}`;
    }
    return undefined;
  }

  private subagentCatalogPrompt(): string {
    const profiles = Object.entries(this.config.profiles)
      .filter(
        ([, profile]) =>
          !profile.disabled && !this.profileUnavailableReason(profile),
      )
      .map(
        ([name, profile]) =>
          `- ${name}: ${profile.description} (tools: ${profile.tools.join(", ") || "none"})`,
      );
    return profiles.length === 0
      ? "No subagent profiles are currently available."
      : [
          "Available subagents for delegate_task:",
          ...profiles,
          "Choose a profile only when its description matches the bounded task.",
        ].join("\n");
  }

  private async reloadAgents(id: string | undefined): Promise<void> {
    const runtime = this.planMode.runtimeHandle();
    this.config = loadHarnessConfig(
      runtime.session.sessionManager.getCwd(),
    );
    const snapshot = this.publishAgentsState();
    this.response(id, "agents_reload", true, snapshot);
  }

  private startDirectSubagent(
    id: string | undefined,
    request: JsonObject,
  ): void {
    const profile = stringField(request, "profile");
    const task = stringField(request, "task").trim();
    if (!task) throw new Error("Subagent task must not be empty");
    const handle = this.startSubagent({ profile, task, direct: true });
    this.response(id, "subagent_start", true, {
      accepted: true,
      agent: this.publicSubagent(handle.agent),
    });
    void handle.completion.catch(() => undefined);
  }

  private async cancelSubagent(
    id: string | undefined,
    request: JsonObject,
  ): Promise<void> {
    const agentId = stringField(request, "agentId");
    const agent = this.subagents.get(agentId);
    if (!agent) throw new Error(`Subagent is not running: ${agentId}`);
    agent.controller.abort();
    if (agent.session) await agent.session.abort();
    this.response(id, "subagent_cancel", true);
  }

  private async integrateSubagent(
    id: string | undefined,
    request: JsonObject,
  ): Promise<void> {
    const agentId = stringField(request, "agentId");
    const action = enumField(
      request,
      "action",
      ["apply", "resolve", "keep", "discard"] as const,
    );
    const completed = this.completedSubagents.get(agentId);
    if (!completed) {
      throw new Error(`Subagent has no pending worktree result: ${agentId}`);
    }
    let record = completed.record;
    let integrationWarning: string | undefined;
    if (action === "resolve") {
      const handle = await this.resolvePendingSubagent(agentId);
      this.response(id, "subagent_integrate", true, {
        status: "resolving",
        resolver: this.publicSubagent(handle.agent),
      });
      void handle.completion.catch((error) => {
        this.restoreResolutionFailure(agentId, error);
      });
      return;
    }
    if (action === "keep") {
      record = await this.worktrees.keep(record);
      this.completedSubagents.delete(agentId);
    } else if (action === "discard") {
      record = await this.worktrees.discard(record);
      this.completedSubagents.delete(agentId);
      if (
        completed.agent.taskId &&
        this.hasMutableGoalTask(
          completed.agent.taskId,
          completed.agent.goalId,
        )
      ) {
        const taskResult = normalizeTaskResult({
          ...completed.result,
          status: "blocked",
          blockers: [
            "The isolated worktree result was discarded before integration",
          ],
        });
        this.goals.updateTask(
          completed.agent.taskId,
          "blocked",
          taskResult,
        );
        this.sendGoalState(
          this.goals.transition(
            "blocked",
            `Goal task ${completed.agent.taskId} was discarded`,
          ),
        );
      }
    } else {
      const result = await this.worktrees.integrate(record);
      record = result.record;
      if (result.status !== "applied") {
        completed.record = record;
        completed.agent.integrationStatus = record.integrationStatus;
        this.send({
          type: "subagent_integration",
          event: record.integrationStatus,
          agent: this.publicSubagent(completed.agent),
          integration: this.worktreeSummary(completed.agent),
          error: result.error,
        });
        this.response(id, "subagent_integrate", true, {
          status: record.integrationStatus,
          integration: this.worktreeSummary(completed.agent),
        });
        this.publishAgentsState();
        return;
      }
      integrationWarning = result.error;
      completed.agent.integrationStatus = "applied";
      this.completedSubagents.delete(agentId);
      if (
        completed.agent.taskId &&
        this.hasMutableGoalTask(
          completed.agent.taskId,
          completed.agent.goalId,
        )
      ) {
        const taskResult = normalizeTaskResult(completed.result);
        this.goals.updateTask(
          completed.agent.taskId,
          taskResult.status,
          taskResult,
        );
        const goal = this.goals.current();
        if (goal?.stage === "paused") {
          const resumed = this.goals.resume();
          this.sendGoalState(resumed);
          if (resumed.stage === "executing") void this.runGoalExecution();
        } else {
          this.sendGoalState();
        }
      }
    }
    completed.record = record;
    completed.agent.integrationStatus = record.integrationStatus;
    this.send({
      type: "subagent_integration",
      event: record.integrationStatus,
      agent: this.publicSubagent(completed.agent),
      integration: this.worktreeSummary(completed.agent),
      ...(integrationWarning ? { error: integrationWarning } : {}),
    });
    this.response(id, "subagent_integrate", true, {
      status: record.integrationStatus,
      integration: this.worktreeSummary(completed.agent),
      ...(integrationWarning ? { warning: integrationWarning } : {}),
    });
    this.publishAgentsState();
  }

  private async resolvePendingSubagent(
    agentId: string,
  ): Promise<SubagentHandle> {
    const completed = this.completedSubagents.get(agentId);
    if (!completed) {
      throw new Error(`Subagent has no pending worktree result: ${agentId}`);
    }
    completed.agent.lifecycle = "resolving";
    if (
      completed.agent.goalId &&
      this.goals.current()?.id === completed.agent.goalId &&
      this.goals.current()?.stage === "paused"
    ) {
      this.sendGoalState(this.goals.resume());
    }
    this.send({
      type: "subagent_integration",
      event: "resolving",
      agent: this.publicSubagent(completed.agent),
      integration: this.worktreeSummary(completed.agent),
    });
    let prepared;
    try {
      prepared = await this.worktrees.prepareResolution(
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
      return this.startSubagent({
        profile: completed.agent.profile,
        task: conflictContext,
        taskId: completed.agent.taskId,
        goalId: completed.agent.goalId,
        outputKind: "task",
        direct: true,
        preparedIsolation: prepared.isolation,
        forceAutoIntegrate: true,
        resolutionForAgentId: agentId,
      });
    } catch (error) {
      try {
        await this.worktrees.discard(prepared.isolation.record);
      } catch (cleanupError) {
        this.reportHostWarning(
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
    if (
      pending.agent.taskId &&
      this.hasMutableGoalTask(pending.agent.taskId, pending.agent.goalId)
    ) {
      this.goals.updateTask(
        pending.agent.taskId,
        "awaiting_integration",
        normalizeTaskResult(pending.result),
      );
      if (this.goals.current()?.stage !== "paused") {
        this.goals.transition(
          "paused",
          `Conflict resolution failed for ${agentId}`,
        );
      }
      this.sendGoalState();
    }
    this.send({
      type: "subagent_integration",
      event: "conflicted",
      agent: this.publicSubagent(pending.agent),
      integration: this.worktreeSummary(pending.agent),
      error: error instanceof Error ? error.message : String(error),
    });
  }

  private runSubagent(options: SubagentOptions): Promise<JsonObject> {
    return this.startSubagent(options).completion;
  }

  private canStartSubagent(profileName: string): boolean {
    if (this.subagents.size >= this.config.maxParallel) return false;
    const profile = this.config.profiles[profileName];
    if (!profile || profile.disabled) return false;
    const activeForProfile = [...this.subagents.values()].filter(
      (agent) => agent.profile === profileName,
    ).length;
    return activeForProfile < profile.maxParallel;
  }

  private waitForSubagentCapacity(): Promise<void> {
    return new Promise((resolvePromise) => {
      this.capacityWaiters.add(resolvePromise);
    });
  }

  private notifySubagentCapacity(): void {
    const waiters = [...this.capacityWaiters];
    this.capacityWaiters.clear();
    for (const waiter of waiters) waiter();
  }

  private startSubagent(options: SubagentOptions): SubagentHandle {
    const goalTask = options.taskId
      ? this.goals
          .active()
          ?.tasks.find((task) => task.id === options.taskId)
      : undefined;
    if (options.taskId && !goalTask) {
      throw new Error(`Unknown active Goal task: ${options.taskId}`);
    }
    if (goalTask) {
      const goal = this.goals.active();
      const incomplete = goalTask.dependsOn.filter(
        (dependency) =>
          goal?.tasks.find((task) => task.id === dependency)?.status !==
          "completed",
      );
      if (incomplete.length > 0) {
        throw new Error(
          `Task ${goalTask.id} is waiting for: ${incomplete.join(", ")}`,
        );
      }
      const allowedStatuses = options.resolutionForAgentId
        ? ["awaiting_integration"]
        : ["pending", "interrupted"];
      if (!allowedStatuses.includes(goalTask.status)) {
        throw new Error(
          `Task ${goalTask.id} cannot start while it is ${goalTask.status}`,
        );
      }
      if (goalTask.profile !== options.profile) {
        throw new Error(
          `Task ${goalTask.id} requires profile ${goalTask.profile}, not ${options.profile}`,
        );
      }
    }
    const profile = this.config.profiles[options.profile];
    if (!profile) {
      throw new Error(`Unknown agent profile: ${options.profile}`);
    }
    if (profile.disabled) {
      throw new Error(`Subagent profile is disabled: ${options.profile}`);
    }
    const unavailable = this.profileUnavailableReason(profile);
    if (unavailable) throw new Error(`Subagent ${options.profile}: ${unavailable}`);
    if (this.subagents.size >= this.config.maxParallel) {
      throw new Error(
        `Subagent concurrency limit reached (${this.config.maxParallel})`,
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
    const runtime = this.planMode.runtimeHandle();
    const cwd =
      options.goalId && this.goals.current()?.id === options.goalId
        ? this.goals.current()!.workspace
        : runtime.session.sessionManager.getCwd();
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
    const agentId = `agent-${++this.subagentSequence}`;
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
      taskId: options.taskId,
      goalId: options.goalId,
      direct: options.direct === true,
      planReadOnly: this.planMode.current() && !options.goalId,
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
    this.send({
      type: "subagent_state",
      event: "queued",
      agent: this.publicSubagent(active),
    });
    this.publishAgentsState();

    const run = async () => {
      active.lifecycle = "preparing_isolation";
      this.send({
        type: "subagent_state",
        event: "preparing_isolation",
        agent: this.publicSubagent(active),
      });
      const prepared =
        options.preparedIsolation ??
        (await this.worktrees.prepare(
          active.id,
          cwd,
          profile.isolation,
          controller.signal,
        ));
      active.isolationBackend = prepared.backend;
      active.isolationWarning = prepared.warning;
      active.worktree = prepared.record;
      if (active.worktree) {
        active.worktree = await this.worktrees.annotate(
          active.worktree,
          this.worktreeRecoveryState(active),
        );
      }
      this.send({
        type: "subagent_state",
        event:
          prepared.backend === "worktree" ? "isolated" : prepared.backend,
        agent: this.publicSubagent(active),
        ...(prepared.warning ? { warning: prepared.warning } : {}),
      });
      const execute = () =>
        this.runSubagentNow(
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
        const sharedExecution = this.writeSubagentTail.then(execute, execute);
        this.writeSubagentTail = sharedExecution.catch(() => undefined);
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
          this.notifySubagentCapacity();
          this.publishAgentsState();
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
          this.publishAgentsState();
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

  private async runSubagentNow(
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
    settings.setProjectTrusted(workspaceIsTrusted(originCwd, this.config));
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
          workspaceIsTrusted(originCwd, this.config),
        ),
      }),
      skillsOverride: (base) => ({
        ...base,
        skills: base.skills.filter((skill) =>
          profile.skills.includes(skill.name),
        ),
      }),
      extensionFactories: [
        this.subagentExtension(agentId, options.profile, profile, model.id),
      ],
    });
    await loader.reload({
      resolveProjectTrust: async () =>
        workspaceIsTrusted(originCwd, this.config),
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
    });
    const session = result.session;
    active.session = session;
    active.lifecycle = "running";
    if (
      options.taskId &&
      this.hasMutableGoalTask(options.taskId, options.goalId)
    ) {
      this.goals.updateTask(options.taskId, "running");
      this.sendGoalState();
    }
    this.send({
      type: "subagent_state",
      event: "started",
      agent: this.publicSubagent(active),
    });
    this.publishAgentsState();

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
      const candidateGoal = options.goalId ? this.goals.current() : undefined;
      const goal =
        candidateGoal?.id === options.goalId ? candidateGoal : undefined;
      const outputInstruction =
        options.outputKind === "goal_spec"
          ? "Return one JSON object only: {summary, acceptanceCriteria, allowedTools, allowedPaths, allowedCommands, tasks:[{id,title,description,profile,dependsOn,allowedPaths,acceptanceCriteria}]}."
          : options.outputKind === "review" || options.profile === "reviewer"
            ? "Return one JSON object only: {verdict, summary, findings}. Each finding should identify affected taskIds and paths when known."
            : "Return one JSON object only: {status, summary, evidence, changedPaths, verification, blockers}.";
      const prompt = [
        `You are Nabla subagent ${agentId} using profile ${options.profile}.`,
        goal
          ? `Goal: ${goal.objective}\nStage: ${goal.stage}\nSpec revision: ${goal.spec?.revision ?? "none"}`
          : "",
        `Assigned task:\n${options.task}`,
        outputInstruction,
      ]
        .filter(Boolean)
        .join("\n\n");
      await session.prompt(prompt);
      if (controller.signal.aborted) throw new Error("Subagent cancelled");
      const text = lastAssistantText(finalMessages);
      const parsed = parseSubagentOutput(text, options.outputKind ?? "task");
      let completed: JsonObject = {
        ...parsed,
        agentId,
        profile: options.profile,
        model: `${model.provider}/${model.id}`,
      };
      let integrationPending = false;
      if (active.worktree) {
        const captured = await this.worktrees.capture(
          active.worktree,
          controller.signal,
        );
        active.worktree = captured.record;
        this.validateWorktreePaths(
          captured.record,
          profile,
          originCwd,
          options.taskId,
          options.goalId,
        );
        if (options.resolutionForAgentId) {
          await this.worktrees.assertResolved(captured.record);
        }
        if (options.discardWorktreeChanges) {
          if (captured.hasChanges) {
            active.worktree = await this.worktrees.discard(captured.record);
            active.integrationStatus = "discarded";
            throw new Error(
              `Verification modified isolated files: ${captured.record.changedPaths.join(", ")}`,
            );
          }
          const integration = await this.worktrees.integrate(captured.record);
          active.worktree = integration.record;
          active.integrationStatus = integration.record.integrationStatus;
          if (integration.status !== "applied") {
            throw new Error(
              integration.error ?? "Unable to close the verification worktree",
            );
          }
          if (integration.error) this.reportHostWarning(integration.error);
        }
        const autoIntegrate =
          !options.discardWorktreeChanges &&
          (!captured.hasChanges ||
            (options.forceAutoIntegrate === true ||
              profile.isolation.integration === "auto" ||
              (profile.isolation.integration === "source" &&
                options.goalId !== undefined)));
        if (autoIntegrate) {
          const integration = await this.worktrees.integrate(
            captured.record,
            controller.signal,
          );
          active.worktree = integration.record;
          active.integrationStatus = integration.record.integrationStatus;
          if (integration.status !== "applied") {
            integrationPending = true;
            active.lifecycle = "awaiting_integration";
            active.worktree = await this.worktrees.annotate(
              integration.record,
              this.worktreeRecoveryState(active, completed),
            );
            this.completedSubagents.set(agentId, {
              agent: active,
              result: completed,
              record: active.worktree,
            });
            if (
              options.taskId &&
              this.hasMutableGoalTask(options.taskId, options.goalId)
            ) {
              this.goals.updateTask(
                options.taskId,
                "awaiting_integration",
                normalizeTaskResult(completed),
              );
              this.goals.transition(
                "paused",
                integration.status === "conflicted"
                  ? `Goal task ${options.taskId} has an integration conflict`
                  : `Goal task ${options.taskId} requires integration reconciliation`,
              );
              this.sendGoalState();
              if (integration.status === "conflicted") {
                setTimeout(() => {
                  void this.resolvePendingSubagent(agentId)
                    .then((handle) => handle.completion)
                    .catch((error) => {
                      this.restoreResolutionFailure(agentId, error);
                    });
                }, 0);
              }
            }
          } else if (integration.error) {
            this.reportHostWarning(integration.error);
          }
        } else {
          integrationPending = captured.hasChanges;
          active.integrationStatus = captured.record.integrationStatus;
          if (integrationPending) {
            active.lifecycle = "awaiting_integration";
            active.worktree = await this.worktrees.annotate(
              captured.record,
              this.worktreeRecoveryState(active, completed),
            );
            this.completedSubagents.set(agentId, {
              agent: active,
              result: completed,
              record: active.worktree,
            });
          }
        }
        completed = {
          ...completed,
          integration: this.worktreeSummary(active),
        };
        this.send({
          type: "subagent_integration",
          event: active.integrationStatus,
          agent: this.publicSubagent(active),
          integration: this.worktreeSummary(active),
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
        const source = this.completedSubagents.get(
          options.resolutionForAgentId,
        );
        if (source) {
          source.record = await this.worktrees.resolvedBy(
            source.record,
            active.id,
          );
          source.agent.integrationStatus = "applied";
          this.completedSubagents.delete(options.resolutionForAgentId);
          this.send({
            type: "subagent_integration",
            event: "applied",
            agent: this.publicSubagent(source.agent),
            integration: this.worktreeSummary(source.agent),
            resolvedBy: active.id,
          });
        }
      }
      if (
        options.taskId &&
        !integrationPending &&
        this.hasMutableGoalTask(options.taskId, options.goalId)
      ) {
        const taskResult = normalizeTaskResult(completed);
        this.goals.updateTask(options.taskId, taskResult.status, taskResult);
        this.sendGoalState();
        if (
          options.resolutionForAgentId &&
          this.goals.active()?.stage === "executing"
        ) {
          void this.runGoalExecution();
        }
      }
      if (active.direct) await this.injectDirectSubagentResult(active, completed);
      this.finishSubagent(
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
            const captured = await this.worktrees.capture(active.worktree);
            active.worktree = captured.record;
            active.integrationStatus = captured.record.integrationStatus;
            if (captured.hasChanges) {
              active.lifecycle = "awaiting_integration";
              active.worktree = await this.worktrees.annotate(
                captured.record,
                this.worktreeRecoveryState(active, limited),
              );
              this.completedSubagents.set(agentId, {
                agent: active,
                result: limited,
                record: active.worktree,
              });
              this.send({
                type: "subagent_integration",
                event: "pending",
                agent: this.publicSubagent(active),
                integration: this.worktreeSummary(active),
                error: String(limited.summary),
              });
            } else {
              await this.worktrees.integrate(captured.record);
            }
          } catch (recoveryError) {
            this.reportHostWarning(
              `Unable to capture worktree changes for ${agentId} after its turn limit: ${
                recoveryError instanceof Error
                  ? recoveryError.message
                  : String(recoveryError)
              }. The registered checkout was preserved for recovery.`,
            );
          }
        }
        if (
          options.taskId &&
          this.hasMutableGoalTask(options.taskId, options.goalId)
        ) {
          this.goals.updateTask(options.taskId, "blocked", {
            status: "blocked",
            summary: String(limited.summary),
            evidence: [],
            changedPaths: [],
            verification: [],
            blockers: [`maxTurns ${profile.maxTurns} reached`],
          });
          this.sendGoalState();
        }
        if (active.direct) {
          await this.injectDirectSubagentResult(active, limited);
        }
        this.finishSubagent(active, "limit_reached", limited);
        return limited;
      }
      const message = error instanceof Error ? error.message : String(error);
      if (options.resolutionForAgentId && active.worktree) {
        try {
          await this.worktrees.discard(active.worktree);
          active.integrationStatus = "discarded";
        } catch (cleanupError) {
          this.reportHostWarning(
            `Unable to discard failed resolver worktree ${active.worktree.id}: ${
              cleanupError instanceof Error
                ? cleanupError.message
                : String(cleanupError)
            }`,
          );
        }
      } else if (active.worktree && active.integrationStatus === "none") {
        try {
          const captured = await this.worktrees.capture(active.worktree);
          active.worktree = captured.record;
          active.integrationStatus = captured.record.integrationStatus;
          if (captured.hasChanges) {
            const failedResult: JsonObject = {
              status: "failed",
              summary: message,
              blockers: [message],
              integration: this.worktreeSummary(active),
            };
            active.lifecycle = "awaiting_integration";
            active.worktree = await this.worktrees.annotate(
              captured.record,
              this.worktreeRecoveryState(active, failedResult),
            );
            this.completedSubagents.set(agentId, {
              agent: active,
              result: failedResult,
              record: active.worktree,
            });
            this.send({
              type: "subagent_integration",
              event: "pending",
              agent: this.publicSubagent(active),
              integration: this.worktreeSummary(active),
              error: message,
            });
          } else {
            await this.worktrees.integrate(captured.record);
          }
        } catch (recoveryError) {
          this.reportHostWarning(
            `Unable to capture worktree changes for failed subagent ${agentId}: ${
              recoveryError instanceof Error
                ? recoveryError.message
                : String(recoveryError)
            }. The original execution error was preserved and the checkout remains registered.`,
          );
        }
      }
      if (
        options.taskId &&
        this.hasMutableGoalTask(options.taskId, options.goalId)
      ) {
        if (controller.signal.aborted) {
          this.goals.updateTask(options.taskId, "interrupted");
        } else {
          this.goals.updateTask(options.taskId, "failed", {
            status: "failed",
            summary: message,
            evidence: [],
            changedPaths: [],
            verification: [],
            blockers: [message],
          });
        }
        this.sendGoalState();
      }
      this.finishSubagent(
        active,
        controller.signal.aborted ? "cancelled" : "failed",
        undefined,
        message,
      );
      throw error;
    } finally {
      unsubscribe();
      controller.signal.removeEventListener("abort", abortChild);
      this.subagents.delete(agentId);
      this.notifySubagentCapacity();
      this.publishAgentsState();
    }
  }

  private finishSubagent(
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
    this.send({
      type: "subagent_state",
      event,
      agent: this.publicSubagent(active),
      ...(result ? { result } : {}),
      ...(error ? { error } : {}),
    });
  }

  private async injectDirectSubagentResult(
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

  private subagentExtension(
    agentId: string,
    profileName: string,
    profile: AgentProfile,
    model: string,
  ): InlineExtension {
    return {
      name: `nabla-subagent-${agentId}`,
      factory: (pi) => {
        pi.on("before_agent_start", (event) => ({
          systemPrompt: [
            event.systemPrompt,
            `This is independent subagent ${agentId} (${profileName}).`,
            ...profile.instructions,
            "Do not ask the user directly. Return structured results to the parent agent.",
          ].join("\n\n"),
        }));
        pi.on("tool_call", (event, context) =>
          this.approveTool(event, context.cwd, context.signal, {
            agentId,
            profile: profileName,
            model,
            profileConfig: profile,
            planReadOnly: this.subagents.get(agentId)?.planReadOnly === true,
            goalId: this.subagents.get(agentId)?.goalId,
            allowedPaths: this.goals
              .active()
              ?.tasks.find((task) => task.id === this.subagents.get(agentId)?.taskId)
              ?.allowedPaths,
          }),
        );
      },
    };
  }

  private publicSubagent(agent: ActiveSubagent): ActiveAgentSnapshot {
    return {
      id: agent.id,
      profile: agent.profile,
      task: agent.task,
      taskId: agent.taskId,
      goalId: agent.goalId,
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

  private worktreeSummary(agent: ActiveSubagent): WorktreeIntegrationSnapshot {
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

  private worktreeRecoveryState(
    agent: ActiveSubagent,
    result?: JsonObject,
  ): WorktreeRecoveryState {
    return {
      profile: agent.profile,
      task: agent.task,
      taskId: agent.taskId,
      goalId: agent.goalId,
      direct: agent.direct,
      planReadOnly: agent.planReadOnly,
      model: agent.model,
      originSessionId: agent.originSessionId,
      ...(result ? { result: structuredClone(result) } : {}),
    };
  }

  private validateWorktreePaths(
    record: WorktreeRecord,
    profile: AgentProfile,
    originCwd: string,
    taskId?: string,
    goalId?: string,
  ): void {
    const goal =
      goalId && this.goals.active()?.id === goalId
        ? this.goals.active()
        : undefined;
    const task = goal?.tasks.find((candidate) => candidate.id === taskId);
    for (const path of record.changedPaths) {
      const absolute = resolve(record.repoRoot, path);
      if (isCredentialPath(absolute)) {
        throw new Error(
          `Worktree result changes a credential-like path: ${path}`,
        );
      }
      let workspaceRelative: string;
      try {
        workspaceRelative = workspaceRelativePath(originCwd, absolute);
      } catch {
        throw new Error(`Worktree result changes outside the workspace: ${path}`);
      }
      if (
        task &&
        !pathAllowedByLease(originCwd, workspaceRelative, task.allowedPaths)
      ) {
        throw new Error(
          `Worktree result changes a path outside the Goal task lease: ${workspaceRelative}`,
        );
      }
      const pathTools = ["edit", "write"].filter((tool) =>
        profile.tools.includes(tool),
      );
      if (
        pathTools.length > 0 &&
        pathTools.every(
          (tool) =>
            agentPermissionEffect(profile, tool, workspaceRelative) === "deny",
        ) &&
        !profile.tools.includes("bash")
      ) {
        throw new Error(
          `Profile ${profile.description} denies the changed path: ${workspaceRelative}`,
        );
      }
    }
  }

  private async runGoalExecution(): Promise<void> {
    if (this.goalAutomationRunning) return;
    const generation = this.goalOperationGeneration;
    const goal = this.goals.active();
    if (!goal || goal.stage !== "executing") return;
    this.goalAutomationRunning = true;
    try {
      while (true) {
        const current = this.goals.active();
        if (!current || current.stage !== "executing") return;
        const failed = current.tasks.find(
          (task) => task.status === "failed" || task.status === "blocked",
        );
        if (failed) {
          throw new Error(`Goal task ${failed.id} is ${failed.status}`);
        }
        const runnable = current.tasks.filter(
          (task) =>
            (task.status === "pending" || task.status === "interrupted") &&
            task.dependsOn.every(
              (dependency) =>
                current.tasks.find((candidate) => candidate.id === dependency)
                  ?.status === "completed",
            ),
        );
        if (runnable.length > 0) {
          const handles: SubagentHandle[] = [];
          for (const task of runnable) {
            if (!this.canStartSubagent(task.profile)) continue;
            handles.push(
              this.startSubagent({
                profile: task.profile,
                taskId: task.id,
                goalId: current.id,
                outputKind: "task",
                task: [
                  `Execute Goal task ${task.id}: ${task.title}`,
                  task.description,
                  current.reviews.at(-1)?.verdict === "changes_required"
                    ? `Repair findings:\n${current.reviews
                        .at(-1)!
                        .findings.map(
                          (finding) =>
                            `- [${finding.severity}] ${finding.title}: ${finding.evidence}\n  ${finding.recommendation}`,
                        )
                        .join("\n")}`
                    : "",
                  `Acceptance criteria:\n${task.acceptanceCriteria.map((item) => `- ${item}`).join("\n")}`,
                  `Allowed paths:\n${task.allowedPaths.map((item) => `- ${item}`).join("\n")}`,
                ].join("\n\n"),
              }),
            );
          }
          if (handles.length === 0) {
            await this.waitForSubagentCapacity();
            continue;
          }
          let firstFailure: unknown;
          await Promise.all(
            handles.map(async (handle) => {
              try {
                await handle.completion;
              } catch (error) {
                if (firstFailure === undefined) {
                  firstFailure = error;
                  for (const sibling of handles) {
                    if (sibling !== handle) sibling.agent.controller.abort();
                  }
                }
                throw error;
              }
            }),
          ).catch(() => undefined);
          if (firstFailure !== undefined) {
            await Promise.allSettled(handles.map((handle) => handle.completion));
            throw firstFailure;
          }
          continue;
        }
        if (current.tasks.some((task) => task.status !== "completed")) {
          throw new Error("Goal task graph has unresolved dependencies");
        }
        await this.verifyAndReviewGoal(current.id);
        const reviewed = this.goals.active();
        if (!reviewed || reviewed.stage !== "executing") return;
      }
    } catch (error) {
      if (generation === this.goalOperationGeneration) {
        this.goalFailed(
          `Goal execution failed: ${
            error instanceof Error ? error.message : String(error)
          }`,
        );
      }
    } finally {
      this.goalAutomationRunning = false;
      const current = this.goals.active();
      if (
        generation !== this.goalOperationGeneration &&
        current?.stage === "executing"
      ) {
        void this.runGoalExecution();
      }
    }
  }

  private async verifyAndReviewGoal(goalId: string): Promise<void> {
    const active = this.goals.active();
    if (!active || active.id !== goalId || active.stage !== "executing") return;
    const verifying = this.goals.transition("verifying");
    this.sendGoalState(verifying);
    const verifierResult = await this.runSubagent({
      profile: "verifier",
      goalId,
      outputKind: "task",
      discardWorktreeChanges: true,
      task: [
        `Verify Goal: ${verifying.objective}`,
        `Acceptance criteria:\n${verifying.acceptanceCriteria.map((item) => `- ${item}`).join("\n")}`,
        "Inspect the implementation and run the relevant test/check commands. Do not modify source files.",
      ].join("\n\n"),
    });
    const normalizedVerification = normalizeTaskResult(verifierResult);
    this.goals.addVerification({
      result: normalizedVerification,
      agentId:
        typeof verifierResult.agentId === "string"
          ? verifierResult.agentId
          : "verifier",
      model:
        typeof verifierResult.model === "string"
          ? verifierResult.model
          : undefined,
    });
    const failedCommand = normalizedVerification.verification.find(
      (evidence) => evidence.exitCode !== 0,
    );
    if (normalizedVerification.status !== "completed" || failedCommand) {
      throw new Error(
        normalizedVerification.status !== "completed"
          ? `Verifier reported ${normalizedVerification.status}: ${normalizedVerification.summary}`
          : `Verifier command failed: ${failedCommand?.command}`,
      );
    }

    const reviewing = this.goals.transition("reviewing");
    this.sendGoalState(reviewing);
    const reviewerResult = await this.runSubagent({
      profile: "reviewer",
      goalId,
      outputKind: "review",
      task: [
        `Independently review Goal: ${reviewing.objective}`,
        `Acceptance criteria:\n${reviewing.acceptanceCriteria.map((item) => `- ${item}`).join("\n")}`,
        `Approved Goal specification:\n${reviewing.spec?.summary ?? ""}`,
        `Task results:\n${JSON.stringify(reviewing.tasks.map((task) => ({ id: task.id, status: task.status, result: task.result })))}`,
        `Verification evidence:\n${JSON.stringify(verifierResult)}`,
        "Return verdict pass, changes_required, or blocked with artifact-backed findings. Do not edit files.",
      ].join("\n\n"),
    });
    const review = normalizeGoalReview(reviewerResult);
    const reviewed = this.goals.addReview({
      ...review,
      agentId:
        typeof reviewerResult.agentId === "string"
          ? reviewerResult.agentId
          : "reviewer",
      model:
        typeof reviewerResult.model === "string"
          ? reviewerResult.model
          : undefined,
    });
    this.sendGoalState(reviewed);
    this.send({
      type: "goal_review",
      goalId: reviewed.id,
      review: reviewed.reviews.at(-1),
    });
  }

  private goalFailed(message: string): void {
    const goal = this.goals.active();
    if (
      !goal ||
      ["paused", "blocked", "completed", "cancelled"].includes(goal.stage)
    ) {
      return;
    }
    const blocked = this.goals.transition("blocked", message);
    this.sendGoalState(blocked);
    this.send({ type: "goal_error", goalId: blocked.id, error: message });
  }

  private async listProviders(): Promise<unknown[]> {
    const providers = await Promise.all(
      this.modelRuntime.getProviders().map(async (provider) => {
        const status = await this.modelRuntime.checkAuth(provider.id);
        const methods: JsonObject[] = [];
        if (provider.auth.oauth) {
          methods.push({
            type: "oauth",
            label:
              provider.auth.oauth.loginLabel ??
              provider.auth.oauth.name ??
              "Sign in with an account",
            available: true,
          });
        }
        if (provider.auth.apiKey) {
          methods.push({
            type: "api_key",
            label: provider.auth.apiKey.name ?? "API key",
            available: typeof provider.auth.apiKey.login === "function",
          });
        }
        return {
          id: provider.id,
          name: provider.name,
          configured: status !== undefined,
          configuredType: status?.type,
          configuredSource: status?.source,
          methods,
        };
      }),
    );
    return providers
      .filter((provider) => provider.methods.some((method) => method.available))
      .sort((left, right) => left.name.localeCompare(right.name));
  }

  private startLogin(id: string | undefined, request: JsonObject): void {
    if (!id) throw new Error("auth_login requires an id");
    if (this.activeFlow) throw new Error("Another login flow is already active");

    const flowId = stringField(request, "flowId");
    const providerId = stringField(request, "providerId");
    const authType = stringField(request, "authType") as AuthType;
    if (authType !== "oauth" && authType !== "api_key") {
      throw new Error(`Unsupported authentication type: ${authType}`);
    }
    const provider = this.modelRuntime.getProvider(providerId);
    if (!provider) throw new Error(`Unknown provider: ${providerId}`);
    if (authType === "oauth" && !provider.auth.oauth) {
      throw new Error(`${provider.name} does not support OAuth login`);
    }
    if (authType === "api_key" && !provider.auth.apiKey?.login) {
      throw new Error(`${provider.name} does not support in-app API key login`);
    }

    const flow: ActiveFlow = {
      id: flowId,
      controller: new AbortController(),
      prompts: new AuthPromptQueue(),
      nextPromptId: 1,
    };
    this.activeFlow = flow;

    void this.modelRuntime
      .login(providerId, authType, {
        signal: flow.controller.signal,
        prompt: (prompt) => this.prompt(flow, prompt),
        notify: (event) => this.notify(flow, event),
      })
      .then(async (credential) => {
        const selectedModel = await this.afterLogin(providerId);
        this.send({
          type: "auth_complete",
          flowId,
          providerId,
          credentialType: credential.type,
          selectedModel,
        });
        this.response(id, "auth_login", true, {
          providerId,
          credentialType: credential.type,
          selectedModel,
        });
      })
      .catch((error) => {
        this.response(
          id,
          "auth_login",
          false,
          undefined,
          error instanceof Error ? error.message : String(error),
        );
      })
      .finally(() => {
        if (this.activeFlow === flow) this.activeFlow = undefined;
        this.rejectPrompts(flow, "Login flow ended");
      });
  }

  private prompt(flow: ActiveFlow, prompt: AuthPrompt): Promise<string> {
    const promptId = String(flow.nextPromptId++);
    return flow.prompts.request(
      promptId,
      [prompt.signal, flow.controller.signal],
      () =>
        this.send({
          type: "auth_prompt",
          flowId: flow.id,
          promptId,
          promptType: prompt.type,
          message: prompt.message,
          placeholder: "placeholder" in prompt ? prompt.placeholder : undefined,
          options: prompt.type === "select" ? prompt.options : undefined,
        }),
      () =>
        this.send({
          type: "auth_prompt_cancelled",
          flowId: flow.id,
          promptId,
        }),
    );
  }

  private notify(flow: ActiveFlow, event: AuthEvent): void {
    this.send({
      type: "auth_notify",
      flowId: flow.id,
      event,
    });
  }

  private replyToPrompt(id: string | undefined, request: JsonObject): void {
    const flow = this.activeFlow;
    const flowId = stringField(request, "flowId");
    const promptId = stringField(request, "promptId");
    if (!flow || flow.id !== flowId) throw new Error("Login flow is no longer active");
    const value = stringField(request, "value");
    if (!flow.prompts.reply(promptId, value)) {
      throw new Error("Authentication prompt is no longer active");
    }
    this.response(id, "auth_reply", true);
  }

  private async logout(id: string | undefined, request: JsonObject): Promise<void> {
    const providerId = stringField(request, "providerId");
    await this.modelRuntime.logout(providerId);
    this.response(id, "auth_logout", true);
  }

  private setPlanMode(id: string | undefined, request: JsonObject): void {
    const active = request.active;
    if (typeof active !== "boolean") {
      throw new Error("set_plan_mode requires a boolean active field");
    }
    const activeTools = this.planMode.set(active);
    this.planMode
      .runtimeHandle()
      .session.sessionManager.appendCustomEntry(PLAN_MODE_ENTRY_TYPE, {
        active,
      });
    const state = { active, activeTools };
    this.send({ type: "plan_mode_state", ...state });
    this.response(id, "set_plan_mode", true, state);
  }

  private sendPlanModeState(): void {
    this.send({
      type: "plan_mode_state",
      active: this.planMode.current(),
      activeTools: this.planMode
        .runtimeHandle()
        .session.getActiveToolNames(),
    });
  }

  private replyApproval(id: string | undefined, request: JsonObject): void {
    const approvalId = stringField(request, "approvalId");
    const decision = stringField(request, "decision");
    if (
      decision !== "allow" &&
      decision !== "allow_goal" &&
      decision !== "deny"
    ) {
      throw new Error(`Unsupported approval decision: ${decision}`);
    }
    if (!this.approvals.reply(approvalId, decision)) {
      throw new Error("Approval request is no longer active");
    }
    this.response(id, "approval_reply", true);
  }

  private replyQuestion(id: string | undefined, request: JsonObject): void {
    const requestId = stringField(request, "requestId");
    const rawAnswers = request.answers;
    if (!Array.isArray(rawAnswers)) throw new Error("question_reply requires answers");
    const answers = rawAnswers.map((answer) => {
      if (!isJsonObject(answer)) throw new Error("Invalid question answer");
      const optionId =
        typeof answer.optionId === "string" && answer.optionId.length > 0
          ? answer.optionId
          : undefined;
      return {
        questionId: stringField(answer, "questionId"),
        value: stringField(answer, "value"),
        ...(optionId ? { optionId } : {}),
      } satisfies QuestionAnswer;
    });
    if (!this.questions.reply(requestId, answers)) {
      throw new Error("Question request is no longer active");
    }
    this.response(id, "question_reply", true);
  }

  private restoreActivePlan(sessionManager: SessionManager) {
    return this.plans.restore(sessionManager.getBranch());
  }

  private async openSessionBrowser(id: string | undefined): Promise<void> {
    const runtime = this.planMode.runtimeHandle();
    if (!runtime.session.isIdle) {
      throw new Error("Cannot browse sessions while the agent is running");
    }
    const catalog = new SessionCatalog({
      manager: runtime.session.sessionManager,
      onProgress: (browserId, scope, loaded, total) =>
        this.send({
          type: "session_list_progress",
          browserId,
          scope,
          loaded,
          total,
        }),
    });
    this.sessionCatalogs.set(catalog.browserId, catalog);
    const snapshot = await catalog.query("current", "", "threaded", false);
    this.response(id, "session_browser_open", true, snapshot);
  }

  private async querySessionBrowser(
    id: string | undefined,
    request: JsonObject,
  ): Promise<void> {
    const browserId = stringField(request, "browserId");
    const catalog = this.sessionCatalogs.get(browserId);
    if (!catalog) throw new Error("Session browser is no longer active");
    const scope = enumField(request, "scope", ["current", "all"] as const);
    const sortMode = enumField(
      request,
      "sortMode",
      ["threaded", "recent", "relevance"] as const,
    );
    const query = optionalStringField(request, "query") ?? "";
    const namedOnly = request.namedOnly === true;
    const offset = optionalNonNegativeIntegerField(request, "offset") ?? 0;
    const snapshot = await catalog.query(
      scope,
      query,
      sortMode,
      namedOnly,
      offset,
    );
    this.response(id, "session_browser_query", true, snapshot);
  }

  private closeSessionBrowser(
    id: string | undefined,
    request: JsonObject,
  ): void {
    this.sessionCatalogs.delete(stringField(request, "browserId"));
    this.response(id, "session_browser_close", true);
  }

  private async newSession(id: string | undefined): Promise<void> {
    const runtime = this.planMode.runtimeHandle();
    if (!runtime.session.isIdle) {
      throw new Error("Cannot create a session while the agent is running");
    }
    if (this.planMode.current()) this.planMode.set(false);
    const result = await runtime.newSession();
    if (result.cancelled) {
      this.response(id, "session_new", true, { cancelled: true });
      return;
    }
    this.sessionCatalogs.clear();
    this.response(id, "session_new", true, {
      cancelled: false,
      activation: this.sessionActivation(),
    });
  }

  private async resumeSession(
    id: string | undefined,
    request: JsonObject,
  ): Promise<void> {
    const runtime = this.planMode.runtimeHandle();
    if (!runtime.session.isIdle) {
      throw new Error("Cannot resume a session while the agent is running");
    }
    const sessionPath = stringField(request, "sessionPath");
    const cwdOverride = optionalStringField(request, "cwdOverride");
    const result = await runtime.switchSession(sessionPath, {
      ...(cwdOverride ? { cwdOverride } : {}),
    });
    if (result.cancelled) {
      this.response(id, "session_resume", true, { cancelled: true });
      return;
    }
    this.sessionCatalogs.clear();
    this.response(id, "session_resume", true, {
      cancelled: false,
      activation: this.sessionActivation(),
    });
  }

  private treeState(id: string | undefined, request: JsonObject): void {
    const runtime = this.planMode.runtimeHandle();
    const filterMode = treeFilterField(request);
    const query = optionalStringField(request, "query") ?? "";
    const foldedEntryIds = stringArrayField(request, "foldedEntryIds");
    this.response(
      id,
      "tree_state",
      true,
      buildTreeSnapshot(
        runtime.session.sessionManager,
        filterMode,
        query,
        foldedEntryIds,
      ),
    );
  }

  private setTreeLabel(id: string | undefined, request: JsonObject): void {
    const runtime = this.planMode.runtimeHandle();
    if (!runtime.session.isIdle) {
      throw new Error("Cannot edit tree labels while the agent is running");
    }
    const entryId = stringField(request, "entryId");
    const label = optionalStringField(request, "label")?.trim() || undefined;
    runtime.session.sessionManager.appendLabelChange(entryId, label);
    this.response(id, "tree_label", true);
  }

  private async copyTreeEntry(
    id: string | undefined,
    request: JsonObject,
  ): Promise<void> {
    const runtime = this.planMode.runtimeHandle();
    const entryId = stringField(request, "entryId");
    const entry = runtime.session.sessionManager.getEntry(entryId);
    if (!entry) throw new Error(`Tree entry not found: ${entryId}`);
    const text = copyTextForEntry(entry);
    if (!text) throw new Error("Selected tree entry has no text to copy");
    await copyToClipboard(text);
    this.response(id, "tree_copy", true);
  }

  private async navigateTree(
    id: string | undefined,
    request: JsonObject,
  ): Promise<void> {
    const runtime = this.planMode.runtimeHandle();
    if (!runtime.session.isIdle) {
      throw new Error("Cannot navigate the tree while the agent is running");
    }
    const entryId = stringField(request, "entryId");
    const summarize = request.summarize === true;
    const customInstructions = optionalStringField(
      request,
      "customInstructions",
    );
    const result = await runtime.session.navigateTree(entryId, {
      summarize,
      ...(customInstructions ? { customInstructions } : {}),
      replaceInstructions: false,
    });
    if (result.cancelled) {
      this.response(id, "tree_navigate", true, {
        cancelled: true,
        aborted: result.aborted === true,
      });
      return;
    }

    const restored = this.restoreActivePlan(runtime.session.sessionManager);
    if (restored.recovered && restored.artifact) {
      runtime.session.sessionManager.appendCustomEntry(
        PLAN_ENTRY_TYPE,
        restored.artifact,
      );
    }
    const restoredPlanMode = restorePlanMode(
      runtime.session.sessionManager.getBranch(),
    );
    if (this.planMode.current() !== restoredPlanMode) {
      this.planMode.set(restoredPlanMode);
    }
    this.sendPlanModeState();
    this.send({
      type: "plan_state",
      artifact: restored.artifact ?? null,
    });
    this.sendContextBudget(this.contextBudget.onTreeNavigation());
    this.response(id, "tree_navigate", true, {
      cancelled: false,
      aborted: false,
      editorText: result.editorText,
      activation: this.sessionActivation(),
    });
  }

  private abortTreeNavigation(id: string | undefined): void {
    this.planMode.runtimeHandle().session.abortBranchSummary();
    this.response(id, "tree_abort", true);
  }

  private sessionActivation(): JsonObject {
    const runtime = this.planMode.runtimeHandle();
    const session = runtime.session;
    const manager = session.sessionManager;
    return {
      state: {
        model: session.model,
        thinkingLevel: session.thinkingLevel,
        isStreaming: session.isStreaming,
        isCompacting: session.isCompacting,
        steeringMode: session.steeringMode,
        followUpMode: session.followUpMode,
        sessionFile: session.sessionFile,
        sessionId: session.sessionId,
        sessionName: session.sessionName,
        autoCompactionEnabled: session.autoCompactionEnabled,
        messageCount: session.messages.length,
        pendingMessageCount: session.pendingMessageCount,
      },
      cwd: manager.getCwd(),
      planMode: this.planMode.current(),
      goal: this.goalSnapshot(),
      history: projectSessionHistory(manager.buildContextEntries()),
      plan: this.plans.latest() ?? null,
      context: this.contextSnapshot(),
    };
  }

  private async executePlan(id: string | undefined, fresh: boolean): Promise<void> {
    const artifact = this.plans.latest();
    if (!artifact) throw new Error("No Plan is submitted");
    if (artifact.status !== "submitted") {
      throw new Error(`Plan cannot execute while it is ${artifact.status}`);
    }

    const runtime = this.planMode.runtimeHandle();
    if (!runtime.session.isIdle) throw new Error("Cannot execute a plan while the agent is running");

    if (this.planMode.current()) {
      this.planMode.set(false);
      runtime.session.sessionManager.appendCustomEntry(PLAN_MODE_ENTRY_TYPE, {
        active: false,
      });
      this.sendPlanModeState();
    }
    const executing = this.plans.markExecuting();
    runtime.session.sessionManager.appendCustomEntry(PLAN_ENTRY_TYPE, executing);

    try {
      if (fresh) {
        const parentSession = runtime.session.sessionFile;
        this.replacementPlan = executing;
        const result = await runtime
          .newSession({
            ...(parentSession ? { parentSession } : {}),
            setup: async (sessionManager) => {
              sessionManager.appendCustomEntry(PLAN_ENTRY_TYPE, executing);
              sessionManager.appendCustomEntry(PLAN_MODE_ENTRY_TYPE, {
                active: false,
              });
            },
          })
          .finally(() => {
            this.replacementPlan = undefined;
          });
        if (result.cancelled) {
          throw new Error("Creating a fresh execution session was cancelled");
        }
        this.plans.adopt(executing);
      }

      const target = runtime.session;
      const command = fresh ? "execute_plan_fresh" : "execute_plan_current";
      this.send({ type: "plan_executing", artifact: executing, fresh });
      this.response(id, command, true, {
        artifact: executing,
        sessionId: target.sessionId,
        fresh,
      });

      void target
        .sendCustomMessage(
          {
            customType: PLAN_EXECUTION_MESSAGE_TYPE,
            content: planExecutionPrompt(executing),
            display: false,
            details: {
              planId: executing.id,
              revision: executing.revision,
              fresh,
            },
          },
          { triggerTurn: true },
        )
        .catch((error) => this.executionFailed(executing, error));
    } catch (error) {
      this.executionFailed(executing, error);
      throw error;
    }
  }

  private executionFailed(artifact: PlanArtifactV2, error: unknown): void {
    const latest = this.plans.latest();
    if (
      latest &&
      latest.id === artifact.id &&
      latest.revision === artifact.revision &&
      latest.status === "executing"
    ) {
      const submitted = this.plans.markSubmitted(
        error instanceof Error ? error.message : String(error),
      );
      const runtime = this.planMode.runtimeHandle();
      runtime.session.sessionManager.appendCustomEntry(
        PLAN_ENTRY_TYPE,
        submitted,
      );
      this.send({
        type: "plan_execution_error",
        artifact: submitted,
        error: error instanceof Error ? error.message : String(error),
      });
    }
  }

  private completePlanExecution(): void {
    const artifact = this.plans.latest();
    if (!artifact || artifact.status !== "executing") return;
    const completed = this.plans.markCompleted();
    this.planMode
      .runtimeHandle()
      .session.sessionManager.appendCustomEntry(PLAN_ENTRY_TYPE, completed);
    this.send({ type: "plan_completed", artifact: completed });
  }

  private async approveTool(
    event: ToolCallEvent,
    cwd: string,
    signal: AbortSignal | undefined,
    agent: {
      agentId?: string;
      profile?: string;
      model?: string;
      profileConfig?: AgentProfile;
      planReadOnly?: boolean;
      goalId?: string;
      allowedPaths?: string[];
    } = {},
  ): Promise<ToolCallEventResult | undefined> {
    const toolName = event.toolName;
    const input = event.input as Record<string, unknown>;
    const path =
      typeof input.path === "string" ? input.path : undefined;
    const command =
      typeof input.command === "string" ? input.command : undefined;
    const mutation = MUTATING_TOOL_NAMES.has(toolName);
    const profile = agent.profileConfig;
    if (profile && !profile.tools.includes(toolName)) {
      return {
        block: true,
        reason: `Tool ${toolName} is not exposed to profile ${agent.profile}`,
      };
    }
    const profileEffect = profile
      ? agentPermissionEffect(
          profile,
          toolName,
          agentToolResource(cwd, path, command),
        )
      : undefined;
    if (
      agent.planReadOnly &&
      (toolName === "edit" ||
        toolName === "write" ||
        (toolName === "bash" &&
          (!command || !isSafeReadOnlyCommand(command))))
    ) {
      return {
        block: true,
        reason: "Subagents started from PLAN mode are read-only",
      };
    }
    if (
      agent.agentId &&
      command &&
      isManagedWorktreeCommand(command)
    ) {
      return {
        block: true,
        reason: "Nabla manages subagent worktrees at the host layer",
      };
    }
    if (profileEffect === "deny") {
      return {
        block: true,
        reason: `Profile ${agent.profile} denies ${toolName} for this resource`,
      };
    }
    if (mutation && !agent.agentId && this.planMode.current()) {
      return { block: true, reason: "Mutation tools are disabled in PLAN mode" };
    }

    let reason =
      profileEffect === "ask"
        ? `Profile ${agent.profile} requires approval`
        : mutation
          ? "Tool can change external state"
          : "Sensitive read";
    let risk: "normal" | "high" | "credential" | "outside_workspace" =
      "normal";
    if (path) {
      const pathError = await workspacePathError(cwd, path);
      if (isCredentialPath(resolve(cwd, path))) {
        reason = "Path may contain credentials";
        risk = "credential";
      } else if (pathError) {
        reason = pathError;
        risk = "outside_workspace";
      }
    }
    if (command && (isHighRiskCommand(command) || hasShellControlSyntax(command))) {
      reason = "Command is high-risk or can cross a trust boundary";
      risk = "high";
    }

    const candidateGoal = this.goals.active();
    const activeGoal =
      agent.goalId && candidateGoal?.id === agent.goalId
        ? candidateGoal
        : undefined;
    const lease = activeGoal?.lease;
    const leaseActive =
      activeGoal !== undefined &&
      ["executing", "verifying", "reviewing"].includes(activeGoal.stage);
    const leaseCovers =
      activeGoal &&
      leaseActive &&
      lease &&
      activeGoal.spec &&
      lease.specRevision === activeGoal.spec.revision &&
      leaseAllowsTool(
        toolName,
        cwd,
        path,
        command,
        lease.allowedTools,
        lease.allowedPaths,
        lease.allowedCommands,
        agent.allowedPaths,
      );
    if (risk === "normal") {
      if (activeGoal && leaseCovers) return undefined;
      if (!activeGoal && profileEffect === "allow") return undefined;
      if (!activeGoal && profileEffect === undefined && !mutation) {
        return undefined;
      }
    }

    if (!this.socket || this.socket.destroyed) {
      return { block: true, reason: "Approval UI is not connected" };
    }

    const decision = await this.approvals.request(
      {
        toolCallId: event.toolCallId,
        toolName,
        input: event.input,
        agentId: agent.agentId,
        agentProfile: agent.profile,
        model: agent.model,
        goalId: activeGoal?.id,
        reason,
        risk,
      },
      signal,
      (approvalEvent) => this.send(approvalEvent),
    );

    if (decision === "allow_goal") {
      if (!activeGoal?.lease || !leaseActive) {
        return {
          block: true,
          reason: "There is no active Goal capability lease to extend",
        };
      }
      this.goals.extendLease(toolName, { path, command });
      this.sendGoalState();
      return undefined;
    }
    if (decision !== "allow") {
      return { block: true, reason: "Denied by user" };
    }
    if (activeGoal && !leaseCovers) {
      return {
        block: true,
        reason: "Goal capability lease must be extended before this tool can run",
      };
    }
    return undefined;
  }

  private cancelActiveFlow(reason: string): void {
    const flow = this.activeFlow;
    if (!flow) return;
    flow.controller.abort();
    this.rejectPrompts(flow, reason);
  }

  private rejectPrompts(flow: ActiveFlow, reason: string): void {
    flow.prompts.cancelAll(reason);
  }
}

function toolsForPlanMode(active: boolean): readonly string[] {
  return active ? PLAN_TOOLS : STANDARD_TOOLS;
}

function agentToolResource(
  cwd: string,
  path: string | undefined,
  command: string | undefined,
): string {
  if (command) return command.trim().replace(/\s+/gu, " ");
  if (!path) return "*";
  return workspaceRelativePath(cwd, resolve(cwd, path));
}

function leaseAllowsTool(
  toolName: string,
  cwd: string,
  path: string | undefined,
  command: string | undefined,
  allowedTools: readonly string[],
  allowedPaths: readonly string[],
  allowedCommands: readonly string[],
  taskPaths: readonly string[] | undefined,
): boolean {
  if (!allowedTools.includes(toolName)) return false;
  if (toolName === "bash") {
    return command !== undefined &&
      commandAllowedByLease(command, allowedCommands);
  }
  if (path) {
    return (
      pathAllowedByLease(cwd, path, allowedPaths) &&
      (!taskPaths || pathAllowedByLease(cwd, path, taskPaths))
    );
  }
  return true;
}

function stringField(value: JsonObject, name: string): string {
  const field = value[name];
  if (typeof field !== "string" || field.length === 0) {
    throw new Error(`Missing string field: ${name}`);
  }
  return field;
}

function optionalStringField(
  value: JsonObject,
  name: string,
): string | undefined {
  const field = value[name];
  return typeof field === "string" ? field : undefined;
}

function optionalNonNegativeIntegerField(
  value: JsonObject,
  name: string,
): number | undefined {
  const field = value[name];
  if (field === undefined) return undefined;
  if (!Number.isInteger(field) || (field as number) < 0) {
    throw new Error(`Invalid non-negative integer field: ${name}`);
  }
  return field as number;
}

function stringArrayField(value: JsonObject, name: string): string[] {
  const field = value[name];
  if (field === undefined) return [];
  if (!Array.isArray(field) || !field.every((item) => typeof item === "string")) {
    throw new Error(`Invalid string array field: ${name}`);
  }
  return field;
}

function enumField<const T extends readonly string[]>(
  value: JsonObject,
  name: string,
  choices: T,
): T[number] {
  const field = stringField(value, name);
  if (!choices.includes(field)) {
    throw new Error(`Unsupported ${name}: ${field}`);
  }
  return field as T[number];
}

function commandLane(request: JsonObject): string | undefined {
  const command = typeof request.type === "string" ? request.type : "";
  if (
    command === "auth_login" ||
    command === "auth_cancel" ||
    command === "auth_logout"
  ) {
    return "auth";
  }
  if (
    command === "resource_reload" ||
    command === "workspace_trust" ||
    command === "agents_reload"
  ) {
    return "configuration";
  }
  if (
    command === "goal_start" ||
    command === "goal_action" ||
    command === "goal_approve"
  ) {
    return "goal";
  }
  if (command === "subagent_integrate") {
    const agentId =
      typeof request.agentId === "string" ? request.agentId : "unknown";
    return `integration:${agentId}`;
  }
  if (command === "subagent_start" || command === "subagent_cancel") {
    return "subagents";
  }
  if (
    command === "session_browser_open" ||
    command === "session_browser_query" ||
    command === "session_browser_close"
  ) {
    return "session-browser";
  }
  if (
    command === "set_plan_mode" ||
    command === "queue_clear" ||
    command === "model_set" ||
    command === "thinking_set" ||
    command === "session_new" ||
    command === "session_resume" ||
    command === "tree_label" ||
    command === "tree_navigate" ||
    command === "tree_abort" ||
    command === "execute_plan_current" ||
    command === "execute_plan_fresh"
  ) {
    return "session";
  }
  return undefined;
}

function treeFilterField(value: JsonObject): TreeFilterMode {
  return enumField(
    value,
    "filterMode",
    ["default", "no-tools", "user-only", "labeled-only", "all"] as const,
  );
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

function normalizeTaskResult(value: JsonObject): TaskResult {
  if (
    value.status !== "completed" &&
    value.status !== "failed" &&
    value.status !== "blocked"
  ) {
    throw new Error(`Invalid task result status: ${String(value.status)}`);
  }
  const status = value.status;
  const verification = Array.isArray(value.verification)
    ? value.verification.flatMap((item) => {
        if (!isJsonObject(item) || typeof item.command !== "string") return [];
        return [
          {
            command: item.command,
            exitCode:
              typeof item.exitCode === "number" ? item.exitCode : null,
            output: typeof item.output === "string" ? item.output : "",
            ...(typeof item.fullOutputPath === "string"
              ? { fullOutputPath: item.fullOutputPath }
              : {}),
          },
        ];
      })
    : [];
  return {
    status,
    summary:
      typeof value.summary === "string" && value.summary.trim()
        ? value.summary
        : (() => {
            throw new Error("Task result summary must not be empty");
          })(),
    evidence: stringArray(value.evidence),
    changedPaths: stringArray(value.changedPaths),
    verification,
    blockers: stringArray(value.blockers),
  };
}

function normalizeGoalReview(
  value: JsonObject,
): Omit<GoalReview, "cycle" | "reviewedAt" | "agentId" | "model"> {
  const verdict =
    value.verdict === "pass" ||
    value.verdict === "changes_required" ||
    value.verdict === "blocked"
      ? value.verdict
      : "blocked";
  const findings = Array.isArray(value.findings)
    ? value.findings.flatMap((item) => {
        if (
          !isJsonObject(item) ||
          typeof item.title !== "string" ||
          typeof item.evidence !== "string"
        ) {
          return [];
        }
        const severity: ReviewFinding["severity"] =
          item.severity === "critical" ||
          item.severity === "high" ||
          item.severity === "medium" ||
          item.severity === "low"
            ? item.severity
            : "medium";
        return [
          {
            severity,
            title: item.title,
            evidence: item.evidence,
            ...(typeof item.path === "string" ? { path: item.path } : {}),
            ...(typeof item.line === "number" ? { line: item.line } : {}),
            recommendation:
              typeof item.recommendation === "string"
                ? item.recommendation
                : "Inspect and repair the finding",
            ...(Array.isArray(item.taskIds)
              ? { taskIds: stringArray(item.taskIds) }
              : {}),
            ...(Array.isArray(item.paths)
              ? { paths: stringArray(item.paths) }
              : {}),
          },
        ];
      })
    : [];
  return {
    verdict,
    summary:
      typeof value.summary === "string"
        ? value.summary
        : "Independent review returned no summary",
    findings,
  };
}

function expandHomePath(value: string): string {
  if (value === "~") return homedir();
  if (value.startsWith("~/")) return resolve(homedir(), value.slice(2));
  return value;
}

const socketPath = process.env.NABLA_CONTROL_SOCKET;
if (!socketPath) throw new Error("NABLA_CONTROL_SOCKET is required");

const cwd = process.cwd();
const agentDir = getAgentDir();
const modelRuntime = await ModelRuntime.create();
const planMode = new PlanModeController();
const plans = new PlanStore();
const contextBudget = new ContextBudgetManager();
const startupSettings = SettingsManager.create(cwd, agentDir);
const startupConfig = loadHarnessConfig(cwd);
startupSettings.setProjectTrusted(workspaceIsTrusted(cwd, startupConfig));
const configuredSessionDir =
  (process.env.PI_CODING_AGENT_SESSION_DIR
    ? expandHomePath(process.env.PI_CODING_AGENT_SESSION_DIR)
    : undefined) ?? startupSettings.getSessionDir();
const startupSessionManager = createStartupSessionManager(
  cwd,
  configuredSessionDir,
);
const goals = new GoalStore({
  cwd,
  sessionId: startupSessionManager.getSessionId(),
});
let runtime: AgentSessionRuntime | undefined;
const hostBridge = new HostBridge(
  socketPath,
  modelRuntime,
  planMode,
  plans,
  contextBudget,
  goals,
  startupConfig,
  async (providerId) => {
    const currentRuntime = runtime;
    if (!currentRuntime) return undefined;
    try {
      if (currentRuntime.session.model) return currentRuntime.session.model;

      const available = await modelRuntime.getAvailable(providerId);
      if (available.length === 0) return undefined;
      const settings = currentRuntime.services.settingsManager;
      const defaultModel =
        settings.getDefaultProvider() === providerId
          ? available.find((model) => model.id === settings.getDefaultModel())
          : undefined;
      const selectedModel = defaultModel ?? available[0];
      await currentRuntime.session.setModel(selectedModel);
      return selectedModel;
    } catch {
      // Authentication remains valid even if model selection needs to be done later.
      return undefined;
    }
  },
);

const createRuntime: CreateAgentSessionRuntimeFactory = async ({
  cwd: runtimeCwd,
  sessionManager,
  sessionStartEvent,
}) => {
  const runtimeConfig = loadHarnessConfig(runtimeCwd);
  const settingsManager = SettingsManager.create(runtimeCwd, agentDir);
  settingsManager.setProjectTrusted(
    workspaceIsTrusted(runtimeCwd, runtimeConfig),
  );
  const services = await createAgentSessionServices({
    cwd: runtimeCwd,
    agentDir,
    modelRuntime,
    settingsManager,
    resourceLoaderOptions: {
      noThemes: true,
      noContextFiles: false,
      extensionFactories: [hostBridge.extension()],
      extensionsOverride: (base) => ({
        ...base,
        extensions: base.extensions.filter(
          (extension) =>
            extension.resolvedPath.startsWith("<inline:") ||
            loadHarnessConfig(runtimeCwd).allowedProjectExtensions.some(
              (allowed) =>
                extension.resolvedPath === resolve(runtimeCwd, allowed) ||
                extension.resolvedPath === resolve(agentDir, allowed),
            ),
        ),
      }),
      agentsFilesOverride: (base) => ({
        agentsFiles: filterContextFilesByTrust(
          base.agentsFiles,
          agentDir,
          workspaceIsTrusted(
            runtimeCwd,
            loadHarnessConfig(runtimeCwd),
          ),
        ),
      }),
    },
  });
  const result = await createAgentSessionFromServices({
    services,
    sessionManager,
    sessionStartEvent,
  });
  planMode.restore(
    result.session,
    restorePlanMode(result.session.sessionManager.getBranch()),
  );
  hostBridge.activateWorkspace(runtimeCwd, result.session);
  return {
    ...result,
    services,
    diagnostics: services.diagnostics,
  };
};

runtime = await createAgentSessionRuntime(createRuntime, {
  cwd,
  agentDir,
  sessionManager: startupSessionManager,
});
planMode.attach(runtime);
await hostBridge.listen();

const shutdown = () => {
  void hostBridge.close().finally(() => process.exit(0));
};
process.once("SIGINT", shutdown);
process.once("SIGTERM", shutdown);

await runRpcMode(runtime);
