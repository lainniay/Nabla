import { resolve } from "node:path";

import {
  ModelRuntime,
  SettingsManager,
  createAgentSessionFromServices,
  createAgentSessionServices,
  type CreateAgentSessionRuntimeFactory,
  type ModelRuntime as ModelRuntimeType,
} from "@earendil-works/pi-coding-agent";

import { ContextBudgetManager } from "../context-manager.ts";
import {
  filterContextFilesByTrust,
  loadHarnessConfig,
  workspaceIsTrusted,
} from "../harness.ts";
import { PlanStore, restorePlanMode } from "../plan.ts";
import { HostDiagnostics } from "../diagnostics/host-diagnostics.ts";
import { ControlServer } from "../transport/control-server.ts";
import { CommandRouter } from "../protocol/command-router.ts";
import { HostEventPublisher } from "../protocol/host-event-publisher.ts";
import type { HostEvent } from "../protocol/contracts.ts";
import type { JsonObject } from "../protocol/validation.ts";
import { PlanModeService } from "../runtime/plan-mode-service.ts";
import { RuntimeSupervisor } from "../runtime/runtime-supervisor.ts";
import { sessionActivation } from "../runtime/session-activation.ts";
import { expandHomePath } from "../runtime/path-utils.ts";
import { PiExtensionFactory } from "../runtime/pi-extension-factory.ts";
import { createNablaBashTool } from "../runtime/create-nabla-bash-tool.ts";
import { InteractionBroker } from "../features/interactions/interaction-broker.ts";
import { ModelService } from "../features/models/model-service.ts";
import { AuthService } from "../features/auth/auth-service.ts";
import { WorkspaceService } from "../features/workspace/workspace-service.ts";
import { BootstrapService } from "../features/bootstrap/bootstrap-service.ts";
import { PermissionService } from "../features/permissions/permission-service.ts";
import { SessionService } from "../features/sessions/session-service.ts";
import { SessionBrowserService } from "../features/sessions/session-browser-service.ts";
import { TreeService } from "../features/sessions/tree-service.ts";
import { PlanService } from "../features/plans/plan-service.ts";
import { ContextService } from "../features/context/context-service.ts";
import { IntegrationService } from "../features/subagents/integration-service.ts";
import { SubagentSupervisor } from "../features/subagents/subagent-supervisor.ts";
import { RustSandboxBackend } from "../permissions/execution/rust-sandbox-backend.ts";
import { createAgentCommands } from "../protocol/commands/agent-commands.ts";
import { createAuthCommands } from "../protocol/commands/auth-commands.ts";
import { createBootstrapCommands } from "../protocol/commands/bootstrap-commands.ts";
import { createConfigurationCommands } from "../protocol/commands/configuration-commands.ts";
import { createInteractionCommands } from "../protocol/commands/interaction-commands.ts";
import { createModelCommands } from "../protocol/commands/model-commands.ts";
import { createPermissionCommands } from "../protocol/commands/permission-commands.ts";
import { createPlanCommands } from "../protocol/commands/plan-commands.ts";
import { createSessionCommands } from "../protocol/commands/session-commands.ts";
import { createStartupSessionManager } from "../session-navigation.ts";
import type { HostApp } from "./host-app.ts";
import { HostAppImpl } from "./host-app.ts";

export interface CreateHostAppOptions {
  socketPath: string;
  cwd: string;
  agentDir: string;
  env: NodeJS.ProcessEnv;
  modelRuntime?: ModelRuntimeType;
  supervisor?: RuntimeSupervisor;
  integrations?: IntegrationService;
}

export async function createHostApp(
  options: CreateHostAppOptions,
): Promise<HostApp> {
  const { socketPath, cwd, agentDir, env } = options;
  const modelRuntime = options.modelRuntime ?? (await ModelRuntime.create());
  const planMode = new PlanModeService();
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
  const controlRef: { current?: ControlServer } = {};
  const events = new HostEventPublisher((event) =>
    controlRef.current?.send(event),
  );
  const send = (event: JsonObject) =>
    events.publish(event as unknown as HostEvent);

  let workspace!: WorkspaceService;
  let permissions!: PermissionService;
  let rustSandboxBackend!: RustSandboxBackend;
  let extensionFactory!: PiExtensionFactory;
  const runtime =
    options.supervisor ??
    new RuntimeSupervisor(
      (async ({
        cwd: runtimeCwd,
        sessionManager,
        sessionStartEvent,
      }: Parameters<CreateAgentSessionRuntimeFactory>[0]) => {
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
            extensionFactories: [extensionFactory.create()],
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
                workspaceIsTrusted(runtimeCwd, loadHarnessConfig(runtimeCwd)),
              ),
            }),
          },
        });
        const result = await createAgentSessionFromServices({
          services,
          sessionManager,
          sessionStartEvent,
          customTools: [
            createNablaBashTool(runtimeCwd, {
              permissions,
              sandboxBackend: rustSandboxBackend,
              options: {
                shellPath: startupSettings.getShellPath(),
                commandPrefix: startupSettings.getShellCommandPrefix(),
              },
            }),
          ],
        });
        planMode.restore(
          result.session,
          restorePlanMode(result.session.sessionManager.getBranch()),
        );
        workspace.activate(runtimeCwd, result.session);
        return {
          ...result,
          services,
          diagnostics: services.diagnostics,
        };
      }) as CreateAgentSessionRuntimeFactory,
    );

  const interactions = new InteractionBroker();
  const context = new ContextService(
    contextBudget,
    send,
    (snapshot) => ({
      ...snapshot,
      scopeId: runtime.current().session.sessionId,
    }),
  );
  let subagents!: SubagentSupervisor;
  workspace = new WorkspaceService(
    runtime,
    planMode,
    modelRuntime,
    send,
    startupConfig,
    () => ({
      active: subagents.activeSnapshots(),
      pending: subagents.pendingSnapshots(),
    }),
    () => controlRef.current?.hasConnection() ?? false,
  );
  const integrations =
    options.integrations ??
    new IntegrationService(
      (message) => diagnostics.warn(message),
      () => workspace.configValue(),
    );
  rustSandboxBackend = await RustSandboxBackend.probe();
  permissions = new PermissionService(
    interactions,
    send,
    planMode,
    () => controlRef.current?.hasConnection() ?? false,
    {
      sessionId: () => runtime.current().session.sessionId,
      cwd: () => runtime.current().session.sessionManager.getCwd(),
    },
    { capability: () => rustSandboxBackend.capability },
  );
  const models = new ModelService(modelRuntime, runtime);
  const auth = new AuthService(
    modelRuntime,
    (providerId) => models.selectDefaultModel(providerId),
    send,
  );
  const plans = new PlanService(planStore, modelRuntime, runtime, planMode, send);
  const browser = new SessionBrowserService(runtime, send);
  const activation = () =>
    sessionActivation(
      runtime.current(),
      planMode,
      plans.snapshot(),
      () => context.scopedSnapshot(),
    );
  const sessions = new SessionService(
    runtime,
    planMode,
    () => browser.closeAll(),
    activation,
  );
  const tree = new TreeService(
    runtime,
    planMode,
    planStore,
    send,
    activation,
    () => context.publish(context.onTreeNavigation()),
  );
  subagents = new SubagentSupervisor(
    workspace,
    integrations,
    permissions,
    rustSandboxBackend,
    modelRuntime,
    runtime,
    planMode,
    send,
    (message) => diagnostics.warn(message),
    () => workspace.publishAgentsState(),
  );
  extensionFactory = new PiExtensionFactory({
    planMode,
    plans,
    context,
    interactions,
    subagents,
    permissions,
    workspace,
    send,
  });
  const bootstrap = new BootstrapService(() => ({
    scopeId: runtime.current().session.sessionId,
    sandbox: rustSandboxBackend.status(),
    planMode: {
      active: planMode.current(),
      activeTools: runtime.current().session.getActiveToolNames(),
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
    (context) => controlRef.current?.isCurrent(context) ?? false,
  );
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
  controlRef.current = control;

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
