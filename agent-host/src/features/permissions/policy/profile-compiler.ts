import { ShellAdapter } from "../adapters/shell.ts";
import { evaluatePermission } from "../evaluator.ts";
import { permissionIntentForTool } from "../tool-intent.ts";
import type { AgentProfile } from "../../subagents/profile-model.ts";
import type {
  CapabilityMatcher,
  FileOperation,
  PermissionRule,
  PolicyEffect,
  ToolContext,
} from "../model.ts";
import { canonicalizePath } from "../filesystem/path.ts";
import { READ_ONLY_TOOL_NAMES } from "../shell/rules.ts";

const FILE_OPERATIONS_BY_TOOL: Record<string, readonly FileOperation[]> = {
  read: ["read"],
  grep: ["read"],
  find: ["list"],
  ls: ["list"],
  edit: ["write"],
  write: ["truncate", "write", "create"],
};

/**
 * Compiles the user-facing profile permission config into the single
 * PermissionRule model consumed by PermissionKernel.
 */
export function compileAgentProfileRules(
  profile: AgentProfile,
  workspace: string,
): PermissionRule[] {
  const rules: PermissionRule[] = [];
  let sequence = 0;
  const push = (effect: PolicyEffect, matcher: CapabilityMatcher): void => {
    rules.push({
      id: `agent-profile-${sequence++}`,
      effect,
      source: "managed",
      matcher,
    });
  };
  for (const tool of READ_ONLY_TOOL_NAMES) {
    push("allow", { kind: "tool", tool });
  }
  for (const [tool, entries] of Object.entries(profile.permission)) {
    for (const rule of entries) {
      if (rule.resource === "*") {
        push(rule.effect, { kind: "tool", tool });
        continue;
      }
      const operations = FILE_OPERATIONS_BY_TOOL[tool];
      if (tool === "bash") {
        push(rule.effect, { kind: "shell_command", pattern: rule.resource });
      } else if (operations) {
        for (const operation of operations) {
          push(rule.effect, {
            kind: "file",
            operation,
            path: canonicalizePath(workspace, rule.resource),
            pattern: true,
          });
        }
      } else {
        push(rule.effect, { kind: "tool", tool });
      }
    }
  }
  return rules;
}

/**
 * Evaluates a profile rule set against one tool/path using the canonical
 * PermissionKernel evaluator. Authorization decisions never use this function;
 * it exists for pre-flight checks (e.g. worktree changed paths) that need the
 * same single evaluator without issuing approvals.
 */
export function evaluateProfilePermission(
  profile: AgentProfile,
  tool: string,
  resource: string,
  workspace: string,
): PolicyEffect {
  return evaluateProfileIntent(
    profile,
    tool,
    tool === "bash" ? { command: resource } : { path: resource },
    workspace,
  );
}

export function evaluateProfileToolExposure(
  profile: AgentProfile,
  tool: string,
  workspace = "",
): PolicyEffect {
  return evaluateProfileIntent(profile, tool, {}, workspace);
}

function evaluateProfileIntent(
  profile: AgentProfile,
  tool: string,
  input: Record<string, unknown>,
  workspace: string,
): PolicyEffect {
  const context: ToolContext = {
    requestId: `profile-${tool}`,
    toolCallId: `profile-${tool}`,
    sessionId: "",
    workspaceId: "",
    cwd: workspace,
  };
  const intent = permissionIntentForTool(
    context,
    tool,
    input,
    new ShellAdapter(),
  );
  return evaluatePermission(
    intent,
    compileAgentProfileRules(profile, workspace),
  ).effect;
}
