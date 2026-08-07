import {
  createBashToolDefinition,
  defineTool,
  type BashToolInput,
  type ToolDefinition,
} from "@earendil-works/pi-coding-agent";

import type {
  PermissionService,
  ToolAuthorizationContext,
} from "../features/permissions/permission-service.ts";
import type { RustSandboxBackend } from "../features/permissions/execution/rust-sandbox-backend.ts";

export interface CreateNablaBashToolOptions {
  shellPath?: string;
  commandPrefix?: string;
  exposeSessionEnvironment?: boolean;
}

export interface CreateNablaBashToolDeps {
  permissions: PermissionService;
  sandboxBackend: RustSandboxBackend;
  agent?: ToolAuthorizationContext["agent"];
  options?: CreateNablaBashToolOptions;
}

export function createNablaBashTool(
  cwd: string,
  deps: CreateNablaBashToolDeps,
): ToolDefinition {
  const options = {
    shellPath: deps.options?.shellPath,
    commandPrefix: deps.options?.commandPrefix,
    exposeSessionEnvironment: deps.options?.exposeSessionEnvironment ?? true,
  };
  const base = createBashToolDefinition(cwd, options);

  return defineTool({
    ...base,
    async execute(
      toolCallId: string,
      params: BashToolInput,
      signal: AbortSignal | undefined,
      onUpdate: Parameters<typeof base.execute>[3],
      ctx: Parameters<typeof base.execute>[4],
    ) {
      const agent = deps.agent
        ? {
            ...deps.agent,
            sessionId: ctx.sessionManager.getSessionId(),
          }
        : undefined;
      const authorization = await deps.permissions.authorizeBash({
        toolCallId,
        command: params.command,
        ...(params.timeout === undefined ? {} : { timeout: params.timeout }),
        cwd,
        signal,
        agent,
      });
      if (authorization.decision === "deny") {
        throw new Error(authorization.reason ?? "Denied by permission policy");
      }

      let succeeded = false;
      try {
        const operations = deps.sandboxBackend.operationsFor(
          authorization.sandboxProfile,
        );
        const result = await createBashToolDefinition(cwd, {
          ...options,
          operations,
        }).execute(toolCallId, params, signal, onUpdate, ctx);
        succeeded = true;
        return result;
      } finally {
        deps.permissions.finishBash(toolCallId, succeeded);
      }
    },
  });
}
