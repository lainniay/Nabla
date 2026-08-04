import { realpathSync } from "node:fs";
import { resolve, sep } from "node:path";

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

export function isSafeReadOnlyWorkspaceCommand(
  command: string,
  cwd: string,
): boolean {
  if (!command.trim() || isHighRiskCommand(command)) return false;
  const sanitized = stripSafeNullRedirects(command);
  if (!sanitized) return false;
  const segments = splitSimpleSequence(sanitized);
  if (!segments || segments.length === 0) return false;
  return segments.every((segment) => {
    if (isSafeReadOnlyCommand(segment)) return true;
    const tokens = shellWords(segment);
    if (!tokens || tokens.length === 0) return false;
    const executable = tokens[0]?.split("/").at(-1);
    if (executable === "git") return isSafeReadOnlyGit(tokens, cwd);
    if (executable === "echo") return true;
    if (executable === "cd") {
      return tokens.length === 2 && workspacePath(cwd, tokens[1] ?? "") !== undefined;
    }
    if (executable === "head") {
      const paths = tokens.slice(1).filter((token) => !token.startsWith("-"));
      return paths.length > 0 && paths.every((path) => workspacePath(cwd, path));
    }
    if (executable === "cat") {
      const paths = tokens.slice(1).filter((token) => !token.startsWith("-"));
      return paths.length > 0 && paths.every((path) => workspacePath(cwd, path));
    }
    if (executable === "ls") {
      if (
        tokens.some(
          (token) =>
            token === "-L" ||
            (token.includes("L") && /^-[^-]/u.test(token)) ||
            token === "--dereference",
        )
      ) {
        return false;
      }
      const paths = tokens.slice(1).filter((token) => !token.startsWith("-"));
      return paths.length === 0 || paths.every((path) => workspacePath(cwd, path));
    }
    if (executable === "wc") {
      const arguments_ = tokens.slice(1);
      if (
        arguments_.some(
          (token) =>
            token.startsWith("-") && token !== "-l" && token !== "--lines",
        )
      ) {
        return false;
      }
      const paths = arguments_.filter((token) => !token.startsWith("-"));
      return paths.length > 0 && paths.every((path) => workspacePath(cwd, path));
    }
    if (executable === "sed") {
      if (tokens[1] !== "-n" || tokens.length < 4) return false;
      const paths = tokens.slice(3).filter((token) => !token.startsWith("-"));
      return paths.length > 0 && paths.every((path) => workspacePath(cwd, path));
    }
    return false;
  });
}

export function toolCallCanMutate(
  toolName: string,
  command: string | undefined,
  cwd: string,
): boolean {
  if (!MUTATING_TOOL_NAMES.has(toolName)) return false;
  return !(
    toolName === "bash" &&
    command !== undefined &&
    isSafeReadOnlyWorkspaceCommand(command, cwd)
  );
}

function isSafeReadOnlyGit(tokens: string[], cwd: string): boolean {
  const arguments_ = tokens.slice(1);
  if (arguments_[0] === "-C") {
    const target = arguments_[1];
    if (
      !target ||
      workspacePath(cwd, target) !== canonicalPath(cwd)
    ) {
      return false;
    }
    arguments_.splice(0, 2);
  }
  const subcommand = arguments_[0];
  const rest = arguments_.slice(1);
  if (!subcommand) return false;
  if (["status", "rev-parse", "ls-files", "describe"].includes(subcommand)) {
    return true;
  }
  if (["diff", "log", "show"].includes(subcommand)) {
    return !rest.some(
      (argument) =>
        argument === "--no-index" ||
        argument === "--output" ||
        argument.startsWith("--output=") ||
        argument === "-o" ||
        argument === "--ext-diff" ||
        argument === "--textconv",
    );
  }
  if (subcommand === "remote") {
    return (
      (rest.length === 1 && ["-v", "--verbose"].includes(rest[0] ?? "")) ||
      (rest[0] === "get-url" &&
        rest.length >= 2 &&
        !rest.includes("--push"))
    );
  }
  if (subcommand === "branch") {
    const mutatingBranchOptions = new Set([
      "-d",
      "-D",
      "-m",
      "-M",
      "-c",
      "-C",
      "--delete",
      "--move",
      "--copy",
      "--edit-description",
      "--set-upstream-to",
      "--unset-upstream",
    ]);
    if (rest.some((argument) => mutatingBranchOptions.has(argument))) {
      return false;
    }
    return (
      rest.length === 0 ||
      rest[0] === "--list" ||
      (rest.length === 1 && rest[0] === "--show-current")
    );
  }
  return subcommand === "worktree" && rest[0] === "list";
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

function stripSafeNullRedirects(command: string): string | undefined {
  const sanitized = command.replace(
    /(?:^|\s)2>\s*\/dev\/null(?=\s|[;&|]|$)/gu,
    " ",
  );
  return /[<>]/u.test(sanitized) ? undefined : sanitized;
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

function splitSimpleSequence(command: string): string[] | undefined {
  const segments: string[] = [];
  let current = "";
  let quote: "'" | "\"" | undefined;
  for (let index = 0; index < command.length; index += 1) {
    const character = command[index]!;
    if (character === "\\" && quote !== "'") {
      const next = command[index + 1];
      if (next === undefined || next === "\n" || next === "\r") return undefined;
      current += character + next;
      index += 1;
      continue;
    }
    if (character === "'" || character === "\"") {
      if (quote === character) quote = undefined;
      else if (!quote) quote = character;
      current += character;
      continue;
    }
    if (quote !== "'" && (character === "$" || character === "`")) {
      return undefined;
    }
    if (!quote) {
      if (character === "|") {
        if (command[index + 1] !== "|" || !current.trim()) return undefined;
        segments.push(current.trim());
        current = "";
        index += 1;
        continue;
      }
      if ("<>\r\n".includes(character)) return undefined;
      if (character === "&") {
        if (command[index + 1] !== "&") return undefined;
        if (!current.trim()) return undefined;
        segments.push(current.trim());
        current = "";
        index += 1;
        continue;
      }
      if (character === ";") {
        if (!current.trim()) return undefined;
        segments.push(current.trim());
        current = "";
        continue;
      }
    }
    current += character;
  }
  if (quote || !current.trim()) return undefined;
  segments.push(current.trim());
  return segments;
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
