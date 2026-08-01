export const READ_ONLY_TOOL_NAMES = ["read", "grep", "find", "ls"] as const;

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

export const MUTATING_TOOL_NAMES = new Set(["edit", "write", "bash"]);

export const SAFE_READ_ONLY_COMMAND_PREFIXES = [
  "git status",
  "git diff",
  "git log",
  "git show",
  "cargo test",
  "cargo check",
  "cargo fmt --check",
  "cargo clippy",
  "npm test",
  "npm run test",
  "npm run lint",
] as const;

export function hasShellControlSyntax(command: string): boolean {
  return (
    /[\r\n;&|<>`]/u.test(command) ||
    /\$\(|\$\{/u.test(command) ||
    /\\\r?\n/u.test(command)
  );
}

export function isSafeReadOnlyCommand(command: string): boolean {
  const normalized = command.trim().replace(/\s+/gu, " ");
  if (
    !normalized ||
    hasShellControlSyntax(command) ||
    isHighRiskCommand(normalized)
  ) {
    return false;
  }
  const prefix = SAFE_READ_ONLY_COMMAND_PREFIXES.find(
    (candidate) =>
      normalized === candidate || normalized.startsWith(`${candidate} `),
  );
  if (!prefix) return false;
  if (
    /^(?:git diff|git log|git show)(?:\s|$)/u.test(normalized) &&
    /(?:^|\s)(?:--output(?:=|\s)|-o(?:\s|$)|--ext-diff(?:\s|$)|--textconv(?:\s|$))/u.test(
      normalized,
    )
  ) {
    return false;
  }
  return true;
}

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
 * The host still owns creation/integration; this parser deliberately catches
 * common shell wrappers and `git -C` forms that the old prefix regex missed.
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
