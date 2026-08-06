import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

import {
  type AgentSession,
  type InlineExtension,
  ModelRuntime,
} from "@earendil-works/pi-coding-agent";

import {
  ContextBudgetManager,
  type ContextSnapshot,
} from "./context-manager.ts";
import {
  agentPermissionEffect,
  agentPermissionSummary,
  type AgentProfile,
  type HarnessConfig,
  type ResourceSnapshot,
} from "./harness.ts";
import {
  PLAN_MODE_ENTRY_TYPE,
  PlanStore,
  type PlanArtifact,
} from "./plan.ts";
import {
  type QuestionAnswer,
} from "./questions.ts";
import {
  type SessionBrowserSnapshot,
  type TreeFilterMode,
  type TreeSnapshot,
} from "./session-navigation.ts";
import { CommandRouter } from "./protocol/command-router.ts";
import { createAgentCommands } from "./protocol/commands/agent-commands.ts";
import { createAuthCommands } from "./protocol/commands/auth-commands.ts";
import { createBootstrapCommands } from "./protocol/commands/bootstrap-commands.ts";
import { createConfigurationCommands } from "./protocol/commands/configuration-commands.ts";
import { createInteractionCommands } from "./protocol/commands/interaction-commands.ts";
import { createModelCommands } from "./protocol/commands/model-commands.ts";
import { createPermissionCommands } from "./protocol/commands/permission-commands.ts";
import { createPlanCommands } from "./protocol/commands/plan-commands.ts";
import { createSessionCommands } from "./protocol/commands/session-commands.ts";
import { isJsonObject, type JsonObject } from "./protocol/validation.ts";
import type {
  ActiveAgentSnapshot,
  AgentsSnapshot,
  BootstrapState,
  HostEvent,
} from "./protocol/contracts.ts";
import {
  HostEventPublisher,
  type OutboundHostEvent,
} from "./protocol/host-event-publisher.ts";
import { HostDiagnostics } from "./diagnostics/host-diagnostics.ts";
import { ControlServer } from "./transport/control-server.ts";
import type { LegacyHostOperations } from "./legacy-host-operations.ts";
import type { ThinkingLevel } from "./policy/tool-policy.ts";
import type { WorkspaceGrantSnapshot } from "./permissions/approvals/workspace-store.ts";
import type { PlanExecutionResult } from "./plan-execution.ts";
import { PlanModeService } from "./runtime/plan-mode-service.ts";
import {
  RuntimeSupervisor,
  type SessionTransition,
} from "./runtime/runtime-supervisor.ts";
import { sessionActivation } from "./runtime/session-activation.ts";
import { agentToolResource, permissionIntentForTool } from "./features/permissions/tool-intent.ts";
import { InteractionBroker } from "./features/interactions/interaction-broker.ts";
import { ModelService } from "./features/models/model-service.ts";
import { AuthService } from "./features/auth/auth-service.ts";
import { WorkspaceService } from "./features/workspace/workspace-service.ts";
import { BootstrapService } from "./features/bootstrap/bootstrap-service.ts";
import { PermissionService } from "./features/permissions/permission-service.ts";
import { SessionService } from "./features/sessions/session-service.ts";
import { SessionBrowserService } from "./features/sessions/session-browser-service.ts";
import { TreeService } from "./features/sessions/tree-service.ts";
import { PlanService } from "./features/plans/plan-service.ts";
import { ContextService } from "./features/context/context-service.ts";
import { IntegrationService } from "./features/subagents/integration-service.ts";
import { SubagentSupervisor } from "./features/subagents/subagent-supervisor.ts";
import { PiExtensionFactory } from "./runtime/pi-extension-factory.ts";
import type { ActiveSubagent } from "./features/subagents/subagent-types.ts";
export class HostBridge implements LegacyHostOperations {
  private readonly control: ControlServer;
  private readonly router: CommandRouter;
  private readonly auth: AuthService;
  private readonly workspace: WorkspaceService;
  private readonly bootstrap: BootstrapService;
  private readonly permissions: PermissionService;
  private readonly integrations: IntegrationService;
  private readonly subagents: SubagentSupervisor;
  private readonly extensionFactory: PiExtensionFactory;
  private readonly sessionBrowser: SessionBrowserService;
  private readonly sessionService: SessionService;
  private readonly treeService: TreeService;
  private readonly events: HostEventPublisher;
  private readonly diagnostics = new HostDiagnostics();
  private readonly interactions = new InteractionBroker();
  private readonly plansService: PlanService;
  private readonly context: ContextService;
  private readonly modelRuntime: ModelRuntime;
  private readonly models: ModelService;
  private readonly planMode: PlanModeService;
  private readonly runtime: RuntimeSupervisor;
  private readonly afterLogin: (providerId: string) => Promise<unknown>;
  private agentsRevision = 0;

  constructor(
    socketPath: string,
    modelRuntime: ModelRuntime,
    models: ModelService,
    planMode: PlanModeService,
    runtime: RuntimeSupervisor,
    plans: PlanStore,
    contextBudget: ContextBudgetManager,
    config: HarnessConfig,
    afterLogin: (providerId: string) => Promise<unknown>,
  ) {
    this.modelRuntime = modelRuntime;
    this.models = models;
    this.planMode = planMode;
    this.runtime = runtime;
    this.context = new ContextService(
      contextBudget,
      (event) => this.send(event),
      (snapshot) => this.contextSnapshot(snapshot),
    );
    this.plansService = new PlanService(
      plans,
      this.modelRuntime,
      this.runtime,
      this.planMode,
      (event) => this.send(event),
    );
    this.permissions = new PermissionService(
      this.interactions,
      (event) => this.send(event),
      this.planMode,
      () => this.control.hasConnection(),
      {
        sessionId: () => this.currentScopeId(),
        cwd: () => this.runtime.current().session.sessionManager.getCwd(),
      },
    );
    this.workspace = new WorkspaceService(
      this.runtime,
      this.planMode,
      this.modelRuntime,
      (event) => this.send(event),
      config,
    );
    this.integrations = new IntegrationService(
      (message) => this.diagnostics.warn(message),
      () => this.workspace.configValue(),
    );
    this.subagents = new SubagentSupervisor(
      this.workspace,
      this.integrations,
      this.permissions,
      this.modelRuntime,
      this.runtime,
      this.planMode,
      (event) => this.send(event),
      (message) => this.diagnostics.warn(message),
      () => this.publishAgentsState(),
    );
    this.extensionFactory = new PiExtensionFactory({
      planMode: this.planMode,
      plans: this.plansService,
      context: this.context,
      interactions: this.interactions,
      subagents: this.subagents,
      permissions: this.permissions,
      workspace: this.workspace,
      send: (event) => this.send(event),
    });
    this.bootstrap = new BootstrapService();
    this.sessionBrowser = new SessionBrowserService(
      this.runtime,
      (event) => this.send(event),
    );
    this.sessionService = new SessionService(
      this.runtime,
      this.planMode,
      () => this.sessionBrowser.closeAll(),
      () => this.sessionActivation(),
    );
    this.treeService = new TreeService(
      this.runtime,
      this.planMode,
      plans,
      (event) => this.send(event),
      () => this.sessionActivation(),
      () => this.context.publish(this.context.onTreeNavigation()),
    );
    this.afterLogin = afterLogin;
    this.auth = new AuthService(
      this.modelRuntime,
      this.afterLogin,
      (event) => this.send(event),
    );
    this.events = new HostEventPublisher((event) => this.writeEvent(event));
    this.events.setScopeIdProvider(() => this.tryCurrentScopeId());
    this.router = new CommandRouter(
      [
        ...createAuthCommands(this),
        ...createBootstrapCommands(this),
        ...createConfigurationCommands(this),
        ...createInteractionCommands(this),
        ...createModelCommands(this),
        ...createPermissionCommands(this),
        ...createPlanCommands(this),
        ...createAgentCommands(this),
        ...createSessionCommands(this),
      ],
      (context) => this.control.isCurrent(context),
    );
    this.control = new ControlServer(
      socketPath,
      this.router,
      {
        onConnectionReplaced: () => {
          this.auth.cancel("Host control client replaced");
          this.interactions.cancelAll("Host control client replaced");
        },
        onConnectionClosed: () => {
          this.auth.cancel("Authentication client disconnected");
          this.interactions.cancelAll("Host control client disconnected");
          this.sessionBrowser.closeAll();
        },
      },
    );
  }

  createExtension(): InlineExtension {
    return this.extensionFactory.create();
  }

  async listen(): Promise<void> {
    await this.recoverWorktrees();
    await this.control.listen();
  }

  async close(): Promise<void> {
    this.auth.cancel("Authentication host stopped");
    this.interactions.cancelAll();
    await this.subagents.hostClose();
    await this.control.close();
  }

  private send(message: JsonObject): void {
    this.events.publish(message as HostEvent);
  }

  private writeEvent(event: OutboundHostEvent): void {
    this.control.send(event);
  }

  private sendContextBudget(snapshot: ContextSnapshot): void {
    this.context.publish(snapshot);
  }

  private currentScopeId(): string {
    return this.runtime.current().session.sessionId;
  }

  private tryCurrentScopeId(): string | undefined {
    try {
      return this.currentScopeId();
    } catch {
      return undefined;
    }
  }

  public contextSnapshot(
    snapshot = this.context.snapshot(),
  ): ContextSnapshot {
    return { ...snapshot, scopeId: this.currentScopeId() };
  }

  private reportHostWarning(message: string): void {
    this.diagnostics.warn(message);
    this.send({ type: "host_warning", message });
  }

  public resourceSnapshot(
    session = this.runtime.current().session,
  ): ResourceSnapshot {
    return this.workspace.resourceSnapshot(session);
  }

  public bootstrapState(): BootstrapState {
    const session = this.runtime.current().session;
    return this.bootstrap.snapshot({
      scopeId: session.sessionId,
      planMode: {
        active: this.planMode.current(),
        activeTools: session.getActiveToolNames(),
      },
      artifact: this.plansService.snapshot(),
      resources: this.resourceSnapshot(),
      agents: this.agentsSnapshot(session),
      context: this.contextSnapshot(),
      pendingIntegrations: this.subagents.pendingIntegrations(),
      warnings: [...this.diagnostics.snapshot()],
    });
  }

  private publishWorkspaceState(
    session = this.runtime.current().session,
  ): { resources: ResourceSnapshot; agents: AgentsSnapshot } {
    this.agentsRevision += 1;
    return this.workspace.publishWorkspaceState(
      session,
      this.agentsSnapshot(session),
    );
  }

  public async reloadResources(): Promise<ResourceSnapshot> {
    return this.workspace.reloadResources(() => this.agentsSnapshot());
  }

  activateWorkspace(cwd: string, session?: AgentSession): void {
    if (session && this.control.hasConnection()) {
      this.workspace.activate(cwd, session, () => this.agentsSnapshot(session));
    } else {
      this.workspace.reloadConfig(cwd);
    }
  }

  public async setWorkspaceTrust(trusted: boolean): Promise<ResourceSnapshot> {
    return this.workspace.setWorkspaceTrust(trusted, () => this.agentsSnapshot());
  }

  public clearQueue(): JsonObject {
    return this.sessionService.clearQueue();
  }

  public async listModels(): Promise<{
    current: { provider: string; id: string } | null;
    models: Array<{
      provider: string;
      id: string;
      name: string;
      reasoning: unknown;
      contextWindow: unknown;
    }>;
  }> {
    return this.models.list();
  }

  public async setModel(input: {
    provider: string;
    modelId: string;
  }): Promise<{ provider: string; id: string; name: string }> {
    return this.models.set(input);
  }

  public setThinking(level: ThinkingLevel): JsonObject {
    return this.models.setThinking(level);
  }

  public agentsSnapshot(
    session = this.runtime.current().session,
  ): AgentsSnapshot {
    return {
      scopeId: session.sessionId,
      revision: this.agentsRevision,
      maxParallel: this.workspace.configValue().maxParallel,
      profiles: Object.entries(this.workspace.configValue().profiles).map(([name, profile]) => ({
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
      active: this.subagents.activeSnapshots(),
      pending: this.subagents.pendingSnapshots(),
      diagnostics: this.workspace.configValue().diagnostics,
    };
  }

  private publishAgentsState(
    session = this.runtime.current().session,
  ): AgentsSnapshot {
    this.agentsRevision += 1;
    const snapshot = this.agentsSnapshot(session);
    this.send({ type: "agents_state", snapshot });
    return snapshot;
  }

  private async recoverWorktrees(): Promise<void> {
    const runtime = this.runtime.current();
    const cwd = runtime.session.sessionManager.getCwd();
    const recovered = await this.integrations.recover(cwd);
    for (const { record, metadata, profile } of recovered) {
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
      this.subagents.restoreRecovered(active, result, record);
    }
  }

  private profileUnavailableReason(
    profile: AgentProfile,
    session = this.runtime.current().session,
  ): string | undefined {
    return this.workspace.profileUnavailableReason(profile, session);
  }

  private subagentCatalogPrompt(): string {
    return this.workspace.subagentCatalogPrompt();
  }

  public async reloadAgents(): Promise<AgentsSnapshot> {
    const runtime = this.runtime.current();
    this.workspace.reloadConfig(runtime.session.sessionManager.getCwd());
    const snapshot = this.publishAgentsState();
    return snapshot;
  }

  public startSubagent(input: {
    profile: string;
    task: string;
  }): { accepted: boolean; agent: ActiveAgentSnapshot } {
    return this.subagents.start(input);
  }

  public async cancelSubagent(agentId: string): Promise<void> {
    await this.subagents.cancel(agentId);
  }

  public async integrateSubagent(input: {
    agentId: string;
    action: "apply" | "resolve" | "keep" | "discard";
  }): Promise<JsonObject> {
    return this.subagents.integrate(input);
  }

  public async listProviders(): Promise<unknown[]> {
    return this.auth.listProviders();
  }

  public startLogin(input: {
    flowId: string;
    providerId: string;
    authType: "oauth" | "api_key";
  }): Promise<{
    providerId: string;
    credentialType: string;
    selectedModel: unknown;
  }> {
    return this.auth.startLogin(input);
  }

  public replyToPrompt(input: {
    flowId: string;
    promptId: string;
    value: string;
  }): void {
    this.auth.replyToPrompt(input);
  }

  public cancelLogin(): void {
    this.auth.cancel("Login cancelled");
  }

  public async logout(providerId: string): Promise<void> {
    await this.auth.logout(providerId);
  }

  public setPlanMode(active: boolean): {
    active: boolean;
    activeTools: readonly string[];
  } {
    const session = this.runtime.current().session;
    const activeTools = this.planMode.set(session, active);
    session.sessionManager.appendCustomEntry(PLAN_MODE_ENTRY_TYPE, {
      active,
    });
    const state = { active, activeTools };
    this.send({ type: "plan_mode_state", ...state });
    return state;
  }

  public planState(): { scopeId: string; artifact: PlanArtifact | null } {
    return {
      scopeId: this.currentScopeId(),
      artifact: this.plansService.snapshot(),
    };
  }

  public workspaceApprovalRules(): WorkspaceGrantSnapshot {
    return this.permissions.workspaceRules();
  }

  public revokeApprovalRule(ruleId: string): WorkspaceGrantSnapshot {
    return this.permissions.revokeWorkspaceRule(ruleId);
  }

  public clearApprovalRules(): WorkspaceGrantSnapshot {
    return this.permissions.clearWorkspaceRules();
  }

  private sendPlanModeState(): void {
    this.send({
      type: "plan_mode_state",
      active: this.planMode.current(),
      activeTools: this.runtime.current().session.getActiveToolNames(),
    });
  }

  public replyApproval(input: {
    requestId: string;
    decision: "allow_once" | "allow_session" | "allow_workspace" | "deny";
  }): void {
    this.interactions.replyApproval(input.requestId, input.decision);
  }

  public replyQuestion(input: {
    requestId: string;
    answers: QuestionAnswer[];
  }): void {
    this.interactions.replyQuestion(input.requestId, input.answers);
  }

  public async openSessionBrowser(): Promise<SessionBrowserSnapshot> {
    return this.sessionBrowser.open();
  }

  public async querySessionBrowser(input: {
    browserId: string;
    scope: "current" | "all";
    sortMode: "threaded" | "recent" | "relevance";
    query: string;
    namedOnly: boolean;
    offset: number;
  }): Promise<SessionBrowserSnapshot> {
    return this.sessionBrowser.query(input);
  }

  public closeSessionBrowser(browserId: string): void {
    this.sessionBrowser.close(browserId);
  }

  public async newSession(): Promise<{
    cancelled: boolean;
    activation?: JsonObject;
  }> {
    return this.sessionService.newSession();
  }

  public async resumeSession(input: {
    sessionPath: string;
    cwdOverride?: string;
  }): Promise<{ cancelled: boolean; activation?: JsonObject }> {
    return this.sessionService.resumeSession(input);
  }

  public treeState(input: {
    filterMode: TreeFilterMode;
    query: string;
    foldedEntryIds: string[];
  }): TreeSnapshot {
    return this.treeService.state(input);
  }

  public setTreeLabel(input: { entryId: string; label?: string }): void {
    this.treeService.label(input);
  }

  public async copyTreeEntry(entryId: string): Promise<void> {
    await this.treeService.copy(entryId);
  }

  public async navigateTree(input: {
    entryId: string;
    summarize: boolean;
    customInstructions?: string;
  }): Promise<JsonObject> {
    return this.treeService.navigate(input);
  }

  public abortTreeNavigation(): void {
    this.treeService.abort();
  }

  private sessionActivation(): JsonObject {
    return sessionActivation(
      this.runtime.current(),
      this.planMode,
      this.plansService.snapshot(),
      () => this.contextSnapshot(),
    );
  }

  public async executePlan(
    context: "current" | "fresh",
  ): Promise<PlanExecutionResult> {
    return this.plansService.execute(context);
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
