import {
  ModelRuntime,
  SettingsManager,
} from "@earendil-works/pi-coding-agent";

import { ContextBudgetManager } from "../features/context/engine.ts";
import type { ContextSnapshot } from "../features/context/model.ts";
import {
  loadHarnessConfig,
  workspaceIsTrusted,
} from "../features/workspace/config.ts";
import { PlanStore } from "../features/plans/store.ts";
import { PlanController } from "../features/plans/plan-controller.ts";
import { HostDiagnostics } from "../diagnostics/host-diagnostics.ts";
import { ControlServer } from "../transport/control-server.ts";
import { CommandRouter } from "../protocol/command-router.ts";
import { HostEventPublisher } from "../protocol/host-event-publisher.ts";
import type { HostEvent } from "../protocol/contracts.ts";
import type { JsonObject } from "../protocol/validation.ts";
import { RuntimeSupervisor } from "../runtime/runtime-supervisor.ts";
import { RuntimeHolder } from "../runtime/runtime-holder.ts";
import { createSessionRuntimeFactory } from "../runtime/session-runtime-factory.ts";
import { sessionActivation } from "../runtime/session-activation.ts";
import { expandHomePath } from "../runtime/path-utils.ts";
import { PiExtensionFactory } from "../runtime/pi-extension-factory.ts";
import {
  ConnectionState,
  EventRoute,
  SubagentStateSource,
} from "./composition-ports.ts";
import { InteractionBroker } from "../features/interactions/interaction-broker.ts";
import { ModelService } from "../features/models/model-service.ts";
import { AuthService } from "../features/auth/auth-service.ts";
import { WorkspaceService } from "../features/workspace/workspace-service.ts";
import { BootstrapService } from "../features/bootstrap/bootstrap-service.ts";
import { PermissionService } from "../features/permissions/permission-service.ts";
import { SessionService } from "../features/sessions/session-service.ts";
import { SessionBrowserService } from "../features/sessions/session-browser-service.ts";
import { TreeService } from "../features/sessions/tree-service.ts";
import { IntegrationService } from "../features/subagents/isolation/integration-service.ts";
import { SubagentSupervisor } from "../features/subagents/subagent-supervisor.ts";
import { RustSandboxBackend } from "../features/permissions/execution/rust-sandbox-backend.ts";
import { createAgentCommands } from "../protocol/commands/agent-commands.ts";
import { createAuthCommands } from "../protocol/commands/auth-commands.ts";
import { createBootstrapCommands } from "../protocol/commands/bootstrap-commands.ts";
import { createConfigurationCommands } from "../protocol/commands/configuration-commands.ts";
import { createInteractionCommands } from "../protocol/commands/interaction-commands.ts";
import { createModelCommands } from "../protocol/commands/model-commands.ts";
import { createPermissionCommands } from "../protocol/commands/permission-commands.ts";
import { createPlanCommands } from "../protocol/commands/plan-commands.ts";
import { createSessionCommands } from "../protocol/commands/session-commands.ts";
import { createStartupSessionManager } from "../features/sessions/startup.ts";
import type { HostApp } from "./host-app.ts";
import { HostAppImpl } from "./host-app.ts";

export interface CreateHostAppOptions {
  socketPath: string;
  cwd: string;
  agentDir: string;
  env: NodeJS.ProcessEnv;
  modelRuntime?: ModelRuntime;
  supervisor?: RuntimeSupervisor;
  integrations?: IntegrationService;
}

export async function createHostApp(
  options: CreateHostAppOptions,
): Promise<HostApp> {
  const { socketPath, cwd, agentDir, env } = options;

  // Core services.
  const modelRuntime = options.modelRuntime ?? (await ModelRuntime.create());
  const planStore = new PlanStore();
  const contextBudget = new ContextBudgetManager();
  const startupSettings = SettingsManager.create(cwd, agentDir);
  const startupConfig = loadHarnessConfig(cwd);
  startupSettings.setProjectTrusted(workspaceIsTrusted(cwd, startupConfig));
  const configuredSessionDir =
    (env.PI_CODING_AGENT_SESSION_DIR
      ? expandHomePath(env.PI_CODING_AGENT_SESSION_DIR)
      : undefined) ?? startupSettings.getSessionDir();
  const startupSessionManager = createStartupSessionManager(
    cwd,
    configuredSessionDir,
  );
  const diagnostics = new HostDiagnostics();

  // Late-bound ports assembled before their owners exist.
  const runtimeAccess = new RuntimeHolder();
  const connection = new ConnectionState();
  const eventRoute = new EventRoute();
  const events = new HostEventPublisher(eventRoute.sink);
  const send = (event: JsonObject) =>
    events.publish(event as unknown as HostEvent);
  const subagentState = new SubagentStateSource();
  const interactions = new InteractionBroker();

  // Modules.
  const rustSandboxBackend = await RustSandboxBackend.probe();
  const scopeContext = (snapshot: ContextSnapshot): ContextSnapshot => ({
    ...snapshot,
    scopeId: runtimeAccess.current().session.sessionId,
  });
  const publishContext = (snapshot: ContextSnapshot): void => {
    const policyWarning = contextBudget.takeWarning();
    send({
      type: "context_budget",
      snapshot: scopeContext(snapshot),
      ...(policyWarning ? { policyWarning } : {}),
    });
  };
  const context = {
    snapshot: () => contextBudget.snapshot(),
    scopedSnapshot: () => scopeContext(contextBudget.snapshot()),
    onRuntimeSessionStart: (runtime: {
      sessionManager: { getSessionId(): string };
      getContextUsage(): Parameters<
        ContextBudgetManager["onModelResponse"]
      >[0];
    }): void => {
      contextBudget.onSessionStart(runtime.sessionManager.getSessionId());
      publishContext(contextBudget.onModelResponse(runtime.getContextUsage()));
    },
    filter: (
      messages: Parameters<ContextBudgetManager["filter"]>[0],
      usage: Parameters<ContextBudgetManager["filter"]>[1],
      options: Parameters<ContextBudgetManager["filter"]>[2],
    ): ReturnType<ContextBudgetManager["filter"]> =>
      contextBudget.filter(messages, usage, options),
    onModelResponse: (
      usage: Parameters<ContextBudgetManager["onModelResponse"]>[0],
    ): ContextSnapshot => contextBudget.onModelResponse(usage),
    onCompaction: (
      record: Parameters<ContextBudgetManager["onCompaction"]>[0],
    ): ContextSnapshot => contextBudget.onCompaction(record),
    onTreeNavigation: (): ContextSnapshot => contextBudget.onTreeNavigation(),
    publish: publishContext,
  };
  const plans = new PlanController(planStore, modelRuntime, runtimeAccess, send);
  const permissions = new PermissionService(
    interactions,
    send,
    plans,
    () => connection.hasConnection(),
    {
      sessionId: () => runtimeAccess.current().session.sessionId,
      cwd: () =>
        runtimeAccess.current().session.sessionManager.getCwd(),
    },
    { capability: () => rustSandboxBackend.capability },
  );
  const workspace = new WorkspaceService(
    runtimeAccess,
    modelRuntime,
    send,
    startupConfig,
    () => subagentState.snapshot(),
    () => connection.hasConnection(),
    (session) => plans.reapply(session),
  );
  const integrations =
    options.integrations ??
    new IntegrationService(
      (message) => diagnostics.warn(message),
      () => workspace.configValue(),
    );
  const models = new ModelService(modelRuntime, runtimeAccess);
  const auth = new AuthService(
    modelRuntime,
    (providerId) => models.selectDefaultModel(providerId),
    send,
  );
  const browser = new SessionBrowserService(runtimeAccess, send);
  const activation = () =>
    sessionActivation(
      runtimeAccess.current(),
      plans,
      plans.snapshot(),
      () => context.scopedSnapshot(),
    );
  const sessions = new SessionService(
    runtimeAccess,
    plans,
    () => browser.closeAll(),
    activation,
  );
  const tree = new TreeService(
    runtimeAccess,
    plans,
    activation,
    () => context.publish(context.onTreeNavigation()),
  );
  const subagents = new SubagentSupervisor(
    workspace,
    integrations,
    permissions,
    rustSandboxBackend,
    modelRuntime,
    runtimeAccess,
    plans,
    send,
    (message) => diagnostics.warn(message),
    () => workspace.publishAgentsState(),
  );
  const extensionFactory = new PiExtensionFactory({
    planMode: plans,
    plans,
    context,
    interactions,
    subagents,
    permissions,
    workspace,
    send,
  });
  const bootstrap = new BootstrapService(() => ({
    scopeId: runtimeAccess.current().session.sessionId,
    sandbox: rustSandboxBackend.status(),
    planMode: {
      active: plans.current(),
      activeTools: runtimeAccess.current().session.getActiveToolNames(),
    },
    artifact: plans.snapshot(),
    resources: workspace.resourceSnapshot(),
    agents: workspace.agentsSnapshot(),
    context: context.scopedSnapshot(),
    pendingIntegrations: subagents.pendingIntegrations(),
    warnings: [...diagnostics.snapshot()],
  }));

  const router = new CommandRouter(
    [
      ...createAuthCommands(auth),
      ...createBootstrapCommands(bootstrap),
      ...createConfigurationCommands({
        resourceSnapshot: () => workspace.resourceSnapshot(),
        reloadResources: () => workspace.reloadResources(),
        setWorkspaceTrust: (trusted) => workspace.setWorkspaceTrust(trusted),
        reloadAgents: () => workspace.reloadAgents(),
      }),
      ...createInteractionCommands(interactions),
      ...createModelCommands(models),
      ...createPermissionCommands(permissions),
      ...createPlanCommands(plans),
      ...createAgentCommands({
        agentsState: () => workspace.agentsSnapshot(),
        startSubagent: (input) => subagents.start(input),
        cancelSubagent: (agentId) => subagents.cancel(agentId),
        integrateSubagent: (input) => subagents.integrate(input),
      }),
      ...createSessionCommands({
        contextState: () => context.scopedSnapshot(),
        clearQueue: () => sessions.clearQueue(),
        openSessionBrowser: () => browser.open(),
        querySessionBrowser: (input) => browser.query(input),
        closeSessionBrowser: (browserId) => browser.close(browserId),
        newSession: () => sessions.newSession(),
        resumeSession: (input) => sessions.resumeSession(input),
        treeState: (input) => tree.state(input),
        setTreeLabel: (input) => tree.label(input),
        copyTreeEntry: (entryId) => tree.copy(entryId),
        navigateTree: (input) => tree.navigate(input),
        abortTreeNavigation: () => tree.abort(),
      }),
    ],
    (context) => connection.isCurrent(context),
  );

  const runtime =
    options.supervisor ??
    new RuntimeSupervisor(
      createSessionRuntimeFactory({
        modelRuntime,
        agentDir,
        startupSettings,
        send,
        extensionFactory,
        permissions,
        rustSandboxBackend,
        plans,
        workspace,
      }),
    );
  runtimeAccess.bind(runtime);
  subagentState.bind(subagents);

  const control = new ControlServer(socketPath, router, {
    onConnectionReplaced: () => {
      auth.cancel("Host control client replaced");
      interactions.cancelAll("Host control client replaced");
      browser.closeAll();
    },
    onConnectionClosed: () => {
      auth.cancel("Authentication client disconnected");
      interactions.cancelAll("Host control client disconnected");
      browser.closeAll();
    },
  });
  connection.bind(control);
  eventRoute.bind((event) => control.send(event));

  return new HostAppImpl(
    runtime,
    control,
    integrations,
    subagents,
    auth,
    interactions,
    startupSessionManager,
    cwd,
    agentDir,
  );
}
