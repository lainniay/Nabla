import { resolve } from "node:path";

import type { AgentProfile } from "../../subagents/profile-model.ts";
import type {
  CapabilityMatcher,
  FileOperation,
  PermissionRule,
  PolicyEffect,
} from "../model.ts";
import { patternMatches } from "../filesystem/path.ts";
import { READ_ONLY_TOOL_NAMES } from "../shell/rules.ts";

const FILE_OPERATION_BY_TOOL: Record<string, FileOperation> = {
  read: "read",
  grep: "read",
  find: "list",
  ls: "list",
  edit: "write",
  write: "write",
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
      const operation = FILE_OPERATION_BY_TOOL[tool];
      if (tool === "bash") {
        push(rule.effect, { kind: "shell_command", pattern: rule.resource });
      } else if (operation) {
        push(rule.effect, {
          kind: "file",
          operation,
          path: absolutePattern(workspace, rule.resource),
          pattern: true,
        });
      } else {
        push(rule.effect, { kind: "tool", tool });
      }
    }
  }
  return rules;
}

/**
 * Display/validation convenience over the same compiled rules. Authorization
 * decisions always go through PermissionKernel.
 */
export function profileToolEffect(
  profile: AgentProfile,
  tool: string,
  resource?: string,
  workspace?: string,
): PolicyEffect {
  let effect: PolicyEffect | undefined;
  for (const rule of compileAgentProfileRules(profile, workspace ?? "")) {
    if (!matchesProfileRule(rule.matcher, tool, resource, workspace)) {
      continue;
    }
    if (rule.effect === "deny") return "deny";
    if (rule.effect === "ask") effect = "ask";
    else if (effect === undefined) effect = "allow";
  }
  return effect ?? (READ_ONLY_TOOL_NAMES.includes(
    tool as (typeof READ_ONLY_TOOL_NAMES)[number],
  )
    ? "allow"
    : "ask");
}

function matchesProfileRule(
  matcher: CapabilityMatcher,
  tool: string,
  resource: string | undefined,
  workspace: string | undefined,
): boolean {
  if (matcher.kind === "tool") return matcher.tool === tool;
  if (resource === undefined) return false;
  if (matcher.kind === "shell_command") {
    return patternMatches(matcher.pattern, resource);
  }
  if (matcher.kind === "file") {
    return workspace !== undefined &&
      patternMatches(matcher.path, resolve(workspace, resource));
  }
  return false;
}

function absolutePattern(workspace: string, pattern: string): string {
  return pattern.startsWith("/") ? pattern : resolve(workspace, pattern);
}
