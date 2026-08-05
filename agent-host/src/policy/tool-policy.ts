import { realpathSync } from "node:fs";
import { resolve, sep } from "node:path";

export const READ_ONLY_TOOL_NAMES = ["read", "grep", "find", "ls"] as const;
export const MUTATING_TOOL_NAMES = new Set(["edit", "write", "bash"]);

export const THINKING_LEVELS = [
  "off",
  "minimal",
  "low",
  "medium",
  "high",
  "xhigh",
  "max",
] as const;

export type ThinkingLevel = (typeof THINKING_LEVELS)[number];

/**
 * Advisory UI signal only. Permission decisions are made exclusively by the
 * structured permission kernel and never consult this function.
 */
export function isHighRiskCommand(command: string): boolean {
  return [
    /(^|\s)sudo(\s|$)/u,
    /(^|\s)rm\s+-(?:[^\s]*r[^\s]*f|[^\s]*f[^\s]*r)(\s|$)/u,
    /\bgit\s+reset\s+--hard\b/u,
    /\bgit\s+clean\s+-[^\s]*f/u,
    /\b(?:curl|wget)\b/u,
    /\b(?:chmod|chown)\b/u,
    /(?:^|\s)>(?:>?)\s*\/(?:etc|usr|bin|sbin)\//u,
  ].some((pattern) => pattern.test(command));
}

/**
 * Advisory detection for commands that would mutate host-managed worktrees.
 * The permission kernel still performs the authoritative deny.
 */
export function isManagedWorktreeCommand(command: string): boolean {
  const tokens =
    command.match(/"[^"]*"|'[^']*'|[^\s;&|()]+/gu)?.map((token) =>
      token.replace(/^(?:"([\s\S]*)"|'([\s\S]*)')$/u, "$1$2"),
    ) ?? [];
  const actions = new Set([
    "add",
    "remove",
    "prune",
    "move",
    "repair",
    "lock",
    "unlock",
  ]);
  for (let index = 0; index < tokens.length; index += 1) {
    const executable = tokens[index]?.split("/").at(-1);
    if (executable !== "git") continue;
    const worktree = tokens.indexOf("worktree", index + 1);
    if (worktree < 0) continue;
    const action = tokens[worktree + 1];
    if (action && actions.has(action)) return true;
  }
  return false;
}

export function stripRedundantWorkspaceCd(
  command: string,
  cwd: string,
): string {
  const match =
    /^\s*cd\s+((?:"(?:[^"\\]|\\.)*"|'[^']*'|[^\s;&|<>`$]+))\s*&&\s*([\s\S]+)$/u.exec(
      command,
    );
  if (!match) return command;
  const words = shellWords(`cd ${match[1] ?? ""}`);
  const target = words?.[1];
  if (!target) return command;
  const root = canonicalPath(cwd);
  if (workspacePath(root, target) !== root) return command;
  return (match[2] ?? command).trimStart();
}

function shellWords(command: string): string[] | undefined {
  const words: string[] = [];
  let current = "";
  let quote: "'" | "\"" | undefined;
  let started = false;
  for (let index = 0; index < command.length; index += 1) {
    const character = command[index]!;
    if (character === "\\" && quote !== "'") {
      const next = command[index + 1];
      if (next === undefined) return undefined;
      current += next;
      started = true;
      index += 1;
      continue;
    }
    if (character === "'" || character === "\"") {
      if (quote === character) quote = undefined;
      else if (!quote) quote = character;
      else current += character;
      started = true;
      continue;
    }
    if (!quote && /\s/u.test(character)) {
      if (started) words.push(current);
      current = "";
      started = false;
      continue;
    }
    current += character;
    started = true;
  }
  if (quote) return undefined;
  if (started) words.push(current);
  return words;
}

function workspacePath(cwd: string, path: string): string | undefined {
  if (!path || path === "-" || path.startsWith("~") || /[(){}]/u.test(path)) {
    return undefined;
  }
  const root = canonicalPath(cwd);
  const target = canonicalPath(resolve(root, path));
  return target === root || target.startsWith(`${root}${sep}`) ? target : undefined;
}

function canonicalPath(path: string): string {
  try {
    return realpathSync(path);
  } catch {
    return resolve(path);
  }
}
