import { resolve } from "node:path";

import { isPathWithin } from "../filesystem/path.ts";

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

const READ_ONLY_GIT_SUBCOMMANDS = new Set([
  "log",
  "status",
  "diff",
  "show",
  "ls-files",
  "ls-tree",
  "rev-parse",
  "blame",
  "grep",
]);

const BENIGN_SHELL_COMMANDS = new Set(["echo", "head", "tail"]);

export function isReadOnlyGitCommand(
  argv: string[],
  cwd: string,
): boolean {
  let index = 0;
  while (index < argv.length) {
    const argument = argv[index]!;
    if (
      argument === "-c" ||
      argument === "--config" ||
      argument === "--ext-diff" ||
      argument === "--git-dir" ||
      argument === "--work-tree"
    ) {
      return false;
    }
    if (argument === "-C") {
      const target = argv[index + 1];
      if (target === undefined) return false;
      if (!isPathWithin(cwd, resolve(cwd, target))) return false;
      index += 2;
      continue;
    }
    if (argument.startsWith("-")) {
      index += 1;
      continue;
    }
    return READ_ONLY_GIT_SUBCOMMANDS.has(argument);
  }
  return false;
}

export function isBenignShellCommand(executable: string): boolean {
  return BENIGN_SHELL_COMMANDS.has(executable);
}

export function isReadOnlyWorkspaceCommand(
  argv: string[],
  cwd: string,
): boolean {
  let operands: string[] = [];
  let index = 0;
  while (index < argv.length) {
    const argument = argv[index]!;
    if (argument === "--") {
      operands.push(...argv.slice(index + 1));
      break;
    }
    if (argument.startsWith("-")) {
      index += 1;
      continue;
    }
    operands.push(argument);
    index += 1;
  }
  if (operands.length === 0) return true;
  return operands.every((operand) =>
    isPathWithin(cwd, resolve(cwd, operand)),
  );
}

export function isReadOnlyCdCommand(
  argv: string[],
  cwd: string,
): boolean {
  if (argv.length === 0) return false;
  if (argv.length > 1) return false;
  return isPathWithin(cwd, resolve(cwd, argv[0]!));
}

const FIND_MUTATING_ACTIONS = new Set([
  "-exec",
  "-execdir",
  "-ok",
  "-delete",
  "-fls",
  "-fprint",
  "-fprintf",
]);

export function isReadOnlyFindCommand(
  argv: string[],
  cwd: string,
): boolean {
  let index = 0;
  while (argv[index] === "-H" || argv[index] === "-L" || argv[index] === "-P") {
    index += 1;
  }
  let foundPath = false;
  while (index < argv.length && !argv[index]!.startsWith("-")) {
    const path = argv[index]!;
    if (!isPathWithin(cwd, resolve(cwd, path))) return false;
    foundPath = true;
    index += 1;
  }
  if (!foundPath && !isPathWithin(cwd, cwd)) return false;
  return !argv.some((argument) => FIND_MUTATING_ACTIONS.has(argument));
}

const XARGS_READ_ONLY_COMMANDS = new Set(["wc", "ls", "head", "tail", "echo"]);
const WORKSPACE_CARGO_SUBCOMMANDS = new Set(["test", "check", "build"]);

export function isReadOnlyXargsCommand(argv: string[]): boolean {
  let index = 0;
  while (index < argv.length) {
    const argument = argv[index]!;
    if (argument === "--") {
      index += 1;
      break;
    }
    if (
      argument === "-0" ||
      argument === "-r" ||
      argument === "-t" ||
      argument === "-x"
    ) {
      index += 1;
      continue;
    }
    if (
      argument === "-d" ||
      argument === "-E" ||
      argument === "-I" ||
      argument === "-i" ||
      argument === "-L" ||
      argument === "-l" ||
      argument === "-n" ||
      argument === "-P" ||
      argument === "-s"
    ) {
      index += 2;
      continue;
    }
    if (argument.startsWith("-")) return false;
    break;
  }
  const command = argv[index];
  if (command === undefined) return true;
  return XARGS_READ_ONLY_COMMANDS.has(command);
}

export function isWorkspaceCargoCommand(argv: string[]): boolean {
  return WORKSPACE_CARGO_SUBCOMMANDS.has(argv[0] ?? "");
}

export function isDangerousExecCommand(
  executable: string,
  argv: string[],
  cwd: string,
): boolean {
  const name = executable.split("/").at(-1)!;
  if (
    name === "sudo" ||
    name === "rm" ||
    name === "chmod" ||
    name === "chown"
  ) {
    return true;
  }
  if (name === "git") return !isReadOnlyGitCommand(argv, cwd);
  if (name === "find") return !isReadOnlyFindCommand(argv, cwd);
  if (name === "xargs") return !isReadOnlyXargsCommand(argv);
  if (name === "ls" || name === "wc") {
    return !isReadOnlyWorkspaceCommand(argv, cwd);
  }
  if (name === "cd") return !isReadOnlyCdCommand(argv, cwd);
  if (name === "cargo") return !isWorkspaceCargoCommand(argv);
  return false;
}

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
