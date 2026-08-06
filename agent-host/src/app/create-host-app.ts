import { resolve } from "node:path";

import {
  ModelRuntime,
  SettingsManager,
  createAgentSessionFromServices,
  createAgentSessionServices,
  type CreateAgentSessionRuntimeFactory,
} from "@earendil-works/pi-coding-agent";

import { ContextBudgetManager } from "../context-manager.ts";
import {
  filterContextFilesByTrust,
  loadHarnessConfig,
  workspaceIsTrusted,
} from "../harness.ts";
import {
  HostBridge,
} from "../legacy-host-bridge.ts";
import { PlanStore, restorePlanMode } from "../plan.ts";
import { expandHomePath } from "../runtime/path-utils.ts";
import { PlanModeService } from "../runtime/plan-mode-service.ts";
import { RuntimeSupervisor } from "../runtime/runtime-supervisor.ts";
import { createStartupSessionManager } from "../session-navigation.ts";
import type { HostApp } from "./host-app.ts";
import { HostAppImpl } from "./host-app.ts";

export async function createHostApp(
  socketPath: string,
  cwd: string,
  agentDir: string,
  env: NodeJS.ProcessEnv,
): Promise<HostApp> {
  const modelRuntime = await ModelRuntime.create();
  const planMode = new PlanModeService();
  const plans = new PlanStore();
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

  let hostBridge!: HostBridge;
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
            workspaceIsTrusted(runtimeCwd, loadHarnessConfig(runtimeCwd)),
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

  const supervisor = new RuntimeSupervisor(createRuntime);
  hostBridge = new HostBridge(
    socketPath,
    modelRuntime,
    planMode,
    supervisor,
    plans,
    contextBudget,
    startupConfig,
    async (providerId) => {
      const currentRuntime = supervisor.current();
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
        return undefined;
      }
    },
  );

  return new HostAppImpl(
    supervisor,
    hostBridge,
    startupSessionManager,
    cwd,
    agentDir,
  );
}
