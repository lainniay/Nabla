import type { AgentIsolationPolicy } from "./isolation/worktree.ts";
import {
  READ_ONLY_TOOL_NAMES,
  type ThinkingLevel,
} from "../permissions/shell/rules.ts";

export interface AgentPermissionRule {
  resource: string;
  effect: "allow" | "ask" | "deny";
}

export type AgentPermissions = Record<string, AgentPermissionRule[]>;

export interface AgentProfile {
  description: string;
  model?: string;
  thinkingLevel?: ThinkingLevel;
  instructions: string[];
  skills: string[];
  tools: string[];
  permission: AgentPermissions;
  maxParallel: number;
  maxTurns: number;
  isolation: AgentIsolationPolicy;
  disabled: boolean;
  source: string;
}

export interface AgentConfigDiagnostic {
  type: "warning" | "error";
  message: string;
  path?: string;
  profile?: string;
}

export const SUPPORTED_AGENT_TOOLS = new Set([
  ...READ_ONLY_TOOL_NAMES,
  "edit",
  "write",
  "bash",
]);

export function readOnlyPermissions(): AgentPermissions {
  return {
    read: [{ resource: "*", effect: "allow" }],
    grep: [{ resource: "*", effect: "allow" }],
    find: [{ resource: "*", effect: "allow" }],
    ls: [{ resource: "*", effect: "allow" }],
    edit: [{ resource: "*", effect: "deny" }],
    write: [{ resource: "*", effect: "deny" }],
    bash: [{ resource: "*", effect: "deny" }],
  };
}

export function writeAgentPermissions(): AgentPermissions {
  return {
    ...readOnlyPermissions(),
    edit: [{ resource: "*", effect: "ask" }],
    write: [{ resource: "*", effect: "ask" }],
    bash: [{ resource: "*", effect: "ask" }],
  };
}

export function safeCustomPermissions(): AgentPermissions {
  return {
    read: [{ resource: "*", effect: "allow" }],
    grep: [{ resource: "*", effect: "allow" }],
    find: [{ resource: "*", effect: "allow" }],
    ls: [{ resource: "*", effect: "allow" }],
  };
}

export function modelReference(profile: AgentProfile): {
  provider: string;
  id: string;
} | undefined {
  const reference = profile.model?.trim();
  if (!reference) return undefined;
  const slash = reference.indexOf("/");
  if (slash <= 0 || slash === reference.length - 1) {
    throw new Error(`Agent model must use provider/model format: ${reference}`);
  }
  return {
    provider: reference.slice(0, slash),
    id: reference.slice(slash + 1),
  };
}
