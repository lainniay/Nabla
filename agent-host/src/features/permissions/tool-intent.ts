import { existsSync } from "node:fs";
import { resolve } from "node:path";

import { workspaceRelativePath } from "./filesystem/path.ts";
import {
  AppendAdapter,
  CreateAdapter,
  DeleteAdapter,
  EditAdapter,
  ListAdapter,
  ReadAdapter,
  RenameAdapter,
  WriteAdapter,
  type FileToolInput,
} from "./adapters/filesystem.ts";
import {
  ShellAdapter,
  type ShellInput,
} from "./adapters/shell.ts";
import { createIntent } from "./adapters/tool-adapter.ts";
import { AgentAdapter } from "./adapters/agent.ts";
import { McpAdapter } from "./adapters/mcp.ts";
import type {
  PermissionIntent,
  ToolContext,
} from "./model.ts";
import { digestValue } from "./shell/digest.ts";
import type { ShellAnalysis } from "./shell/planner.ts";
import { isJsonObject } from "../../protocol/validation.ts";

export function assessOpaqueRisk(
  intent: PermissionIntent,
  shellAnalysis?: ShellAnalysis,
): boolean {
  return intent.tool === "bash"
    ? (shellAnalysis?.safety.opaque ?? false)
    : intent.atoms.some((atom) => atom.kind === "opaque_code");
}

export function permissionIntentForTool(
  context: ToolContext,
  toolName: string,
  input: unknown,
  shellAdapter: ShellAdapter,
): PermissionIntent {
  const value = isJsonObject(input) ? input : {};
  if (toolName === "delegate_task") {
    return new AgentAdapter().normalize(context, {
      action: "spawn",
      ...(typeof value.profile === "string" ? { profile: value.profile } : {}),
      payload: value,
    });
  }
  if (toolName.startsWith("mcp__")) {
    const [, server = "unknown", ...methodParts] = toolName.split("__");
    return new McpAdapter().normalize(context, {
      server,
      method: methodParts.join("__") || toolName,
      arguments: value,
    });
  }
  if (toolName === "bash" && typeof value.command === "string") {
    return shellAdapter.normalize(context, {
      command: value.command,
      ...(typeof value.cwd === "string" ? { cwd: value.cwd } : {}),
      ...(isStringRecord(value.environment)
        ? { environment: value.environment }
        : {}),
    } satisfies ShellInput);
  }
  if (toolName === "find" || toolName === "grep" || toolName === "ls") {
    const path = typeof value.path === "string" ? value.path : ".";
    const adapter = toolName === "grep" ? ReadAdapter : ListAdapter;
    return adapter.normalize(context, { ...value, path } as FileToolInput);
  }
  if (typeof value.path === "string") {
    const adapter = (() => {
      switch (toolName) {
        case "edit":
        case "edit_file":
          return EditAdapter;
        case "write":
        case "write_file":
          return existsSync(resolve(context.cwd, value.path))
            ? WriteAdapter
            : CreateAdapter;
        case "append":
        case "append_file":
          return AppendAdapter;
        case "rename":
        case "move":
        case "move_file":
          return RenameAdapter;
        case "delete":
        case "remove":
        case "delete_file":
          return DeleteAdapter;
        case "ls":
          return ListAdapter;
        default:
          return ReadAdapter;
      }
    })();
    return adapter.normalize(context, value as FileToolInput);
  }
  const normalizedInput = isJsonObject(input) ? input : { value: input };
  return createIntent(context, toolName, normalizedInput, [{
    kind: "opaque_code",
    runtime: `tool:${toolName}`,
    digest: digestValue(normalizedInput),
    reason: "tool input has no specialized capability adapter",
  }]);
}

export function agentToolResource(
  cwd: string,
  path: string | undefined,
  command: string | undefined,
): string {
  if (command) return command.trim().replace(/\s+/gu, " ");
  if (!path) return "*";
  return workspaceRelativePath(cwd, resolve(cwd, path));
}

function isStringRecord(value: unknown): value is Record<string, string> {
  return (
    isJsonObject(value) &&
    Object.values(value).every((item) => typeof item === "string")
  );
}
