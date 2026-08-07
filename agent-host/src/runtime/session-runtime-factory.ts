import {
  SettingsManager,
  createAgentSessionFromServices,
  createAgentSessionServices,
  type CreateAgentSessionRuntimeFactory,
  type ModelRuntime,
} from "@earendil-works/pi-coding-agent";
import { resolve } from "node:path";

import {
  loadHarnessConfig,
  workspaceIsTrusted,
} from "../features/workspace/config.ts";
import { filterContextFilesByTrust } from "../features/workspace/trust.ts";
import type { WorkspaceService } from "../features/workspace/workspace-service.ts";
import type { PlanController } from "../features/plans/plan-controller.ts";
import type { PermissionService } from "../features/permissions/permission-service.ts";
import type { RustSandboxBackend } from "../features/permissions/execution/rust-sandbox-backend.ts";
import { createNablaBashTool } from "./create-nabla-bash-tool.ts";
import type { PiExtensionFactory } from "./pi-extension-factory.ts";
import type { JsonObject } from "../protocol/validation.ts";

export interface SessionRuntimeFactoryDeps {
  modelRuntime: ModelRuntime;
  agentDir: string;
  startupSettings: SettingsManager;
  send: (event: JsonObject) => void;
  extensionFactory: PiExtensionFactory;
  permissions: PermissionService;
  rustSandboxBackend: RustSandboxBackend;
  plans: PlanController;
  workspace: WorkspaceService;
}

export function createSessionRuntimeFactory(
  deps: SessionRuntimeFactoryDeps,
): CreateAgentSessionRuntimeFactory {
  const {
    modelRuntime,
    agentDir,
    startupSettings,
    send,
    extensionFactory,
    permissions,
    rustSandboxBackend,
    plans,
    workspace,
  } = deps;
  return async ({
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
    plans.activateSession(
      result.session.sessionManager.getBranch(),
      result.session,
    );
    workspace.activate(runtimeCwd, result.session);
    return {
      ...result,
      services,
      diagnostics: services.diagnostics,
    };
  };
}
