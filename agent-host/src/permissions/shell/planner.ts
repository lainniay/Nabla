import { globSync } from "node:fs";
import { isAbsolute, resolve } from "node:path";

import type {
  CapabilityAtom,
  ExecCapability,
  FileCapability,
  OpaqueCodeCapability,
} from "../model.ts";
import type { ShellCommand, ShellScript } from "./ast.ts";
import { digestValue } from "./digest.ts";
import { parseShell } from "./parser.ts";

const OPAQUE_SHELL_WORDS = new Set([
  "if",
  "then",
  "else",
  "elif",
  "fi",
  "for",
  "while",
  "until",
  "do",
  "done",
  "case",
  "esac",
  "function",
  "time",
  "!",
  "eval",
  "source",
  ".",
  "exec",
]);

const ALWAYS_NETWORK_COMMANDS = new Set([
  "curl",
  "wget",
  "nc",
  "ssh",
  "scp",
  "rsync",
]);

const GIT_NETWORK_SUBCOMMANDS = new Set([
  "push",
  "fetch",
  "clone",
  "pull",
  "ls-remote",
  "remote",
]);

const NPM_NETWORK_SUBCOMMANDS = new Set([
  "install",
  "add",
  "publish",
  "update",
  "ci",
]);

const CARGO_NETWORK_SUBCOMMANDS = new Set([
  "publish",
  "install",
  "update",
  "add",
]);

const PIP_NETWORK_SUBCOMMANDS = new Set(["install", "download"]);

export interface ExecutionPlan {
  source: string;
  cwd: string;
  atoms: CapabilityAtom[];
  commands: ExecCapability[];
  globExpansions: Record<string, string[]>;
  readOnly: boolean;
  opaque: boolean;
  requiresShell: boolean;
}

export function planShell(
  source: string,
  cwd: string,
  environment: Record<string, string> = {},
): ExecutionPlan {
  return planParsedShell(parseShell(source), cwd, environment);
}

export function planParsedShell(
  script: ShellScript,
  cwd: string,
  environment: Record<string, string> = {},
): ExecutionPlan {
  const atoms: CapabilityAtom[] = [];
  const commands: ExecCapability[] = [];
  const globExpansions: Record<string, string[]> = {};
  const initialCwd = resolve(cwd);
  let currentCwd = initialCwd;
  let opaque = false;
  let requiresShell =
    script.connectors.length > 0 ||
    script.nodes.some((node) => node.type === "group");

  if (script.opaqueReason) {
    atoms.push(opaqueAtom("shell", script.source, script.opaqueReason));
    return {
      source: script.source,
      cwd: initialCwd,
      atoms,
      commands,
      globExpansions,
      readOnly: false,
      opaque: true,
      requiresShell: true,
    };
  }

  for (const node of script.nodes) {
    if (node.type === "group") {
      const nested = planParsedShell(node.script, currentCwd, environment);
      atoms.push(...nested.atoms);
      commands.push(...nested.commands);
      Object.assign(globExpansions, nested.globExpansions);
      opaque ||= nested.opaque;
      requiresShell ||= nested.requiresShell;
      continue;
    }
    const planned = planCommand(node, currentCwd, environment);
    atoms.push(...planned.atoms);
    commands.push(...planned.commands);
    Object.assign(globExpansions, planned.globExpansions);
    opaque ||= planned.opaque;
    requiresShell ||= node.redirections.length > 0 ||
      node.substitutions.length > 0;
    if (node.argv[0] === "cd" && node.argv.length === 2 && !node.opaqueReason) {
      currentCwd = resolve(currentCwd, node.argv[1]!);
    }
  }
  return {
    source: script.source,
    cwd: initialCwd,
    atoms,
    commands,
    globExpansions,
    readOnly: !opaque && atoms.every((atom) =>
      atom.kind !== "file" ||
      atom.operation === "read" ||
      atom.path === "/dev/null"
    ),
    opaque,
    requiresShell,
  };
}

function planCommand(
  command: ShellCommand,
  cwd: string,
  inheritedEnvironment: Record<string, string>,
): Pick<ExecutionPlan, "atoms" | "commands" | "globExpansions" | "opaque"> {
  const atoms: CapabilityAtom[] = [];
  const commands: ExecCapability[] = [];
  const globExpansions: Record<string, string[]> = {};
  let opaque = false;
  if (command.opaqueReason || command.argv.length === 0) {
    atoms.push(
      opaqueAtom(
        command.argv[0] ?? "shell",
        command.source,
        command.opaqueReason ?? "missing executable",
      ),
    );
    opaque = true;
  } else {
    const executableName = command.argv[0]!.split("/").at(-1)!;
    if (OPAQUE_SHELL_WORDS.has(executableName)) {
      atoms.push(
        opaqueAtom(executableName, command.source, "shell control keyword is opaque"),
      );
      opaque = true;
      return { atoms, commands, globExpansions, opaque };
    }
    const exec: ExecCapability = {
      kind: "exec",
      executable: command.argv[0]!,
      argv: command.argv.slice(1),
      cwd,
      environment: { ...inheritedEnvironment, ...command.assignments },
    };
    atoms.push(exec);
    commands.push(exec);
    const network = networkCapability(executableName, command.argv.slice(1));
    if (network) atoms.push(network);
    for (const operand of fileReadOperands(command)) {
      const expanded = expandOperand(operand, cwd);
      if (expanded.pattern) {
        globExpansions[`${cwd}\u0000${operand}`] = expanded.paths;
      }
      atoms.push(...expanded.paths.map((path): FileCapability => ({
        kind: "file",
        operation: "read",
        path,
      })));
    }

    const executable = command.argv[0]!.split("/").at(-1);
    if ((executable === "bash" || executable === "sh") && command.argv[1] === "-c") {
      const nestedSource = command.argv[2];
      if (
        nestedSource === undefined ||
        /[$`]/u.test(nestedSource)
      ) {
        atoms.push(
          opaqueAtom(executable, command.source, "dynamic interpreter script"),
        );
        opaque = true;
      } else {
        const nested = planShell(nestedSource, cwd, exec.environment);
        atoms.push(...nested.atoms);
        commands.push(...nested.commands);
        opaque ||= nested.opaque;
      }
    }
    if (
      ["python", "python3", "node"].includes(executable ?? "") &&
      command.argv[1] === "-c"
    ) {
      atoms.push(
        opaqueAtom(executable!, command.argv[2] ?? "", "interpreter code"),
      );
      opaque = true;
    }
  }
  for (const redirection of command.redirections) {
    const operation =
      redirection.operation === "read"
        ? "read"
        : redirection.operation === "append"
          ? "append"
          : "write";
    const atom: FileCapability = {
      kind: "file",
      operation,
      path: isAbsolute(redirection.target)
        ? resolve(redirection.target)
        : resolve(cwd, redirection.target),
    };
    atoms.push(atom);
  }
  for (const substitution of command.substitutions) {
    const nested = planParsedShell(substitution, cwd, inheritedEnvironment);
    atoms.push(...nested.atoms);
    commands.push(...nested.commands);
    Object.assign(globExpansions, nested.globExpansions);
    opaque ||= nested.opaque;
  }
  return { atoms, commands, globExpansions, opaque };
}

function networkCapability(
  executable: string,
  argv: string[],
): { kind: "network"; operation: "connect"; host: string } | undefined {
  if (ALWAYS_NETWORK_COMMANDS.has(executable)) {
    return { kind: "network", operation: "connect", host: "*" };
  }
  const subcommand = argv[0];
  if (subcommand === undefined) return undefined;
  if (executable === "git" && GIT_NETWORK_SUBCOMMANDS.has(subcommand)) {
    return { kind: "network", operation: "connect", host: "*" };
  }
  if (
    (executable === "npm" || executable === "pnpm" || executable === "yarn") &&
    NPM_NETWORK_SUBCOMMANDS.has(subcommand)
  ) {
    return { kind: "network", operation: "connect", host: "*" };
  }
  if (executable === "cargo" && CARGO_NETWORK_SUBCOMMANDS.has(subcommand)) {
    return { kind: "network", operation: "connect", host: "*" };
  }
  if (
    (executable === "pip" || executable === "pip3") &&
    PIP_NETWORK_SUBCOMMANDS.has(subcommand)
  ) {
    return { kind: "network", operation: "connect", host: "*" };
  }
  return undefined;
}

function fileReadOperands(command: ShellCommand): string[] {
  const executable = command.argv[0]?.split("/").at(-1);
  const argv = command.argv.slice(1);
  if (executable === "cat") {
    const separator = argv.indexOf("--");
    return argv.filter((value, index) =>
      value !== "-" &&
      (separator >= 0 ? index > separator : !value.startsWith("-"))
    );
  }
  if (executable !== "head") return [];
  const operands: string[] = [];
  for (let index = 0; index < argv.length; index += 1) {
    const value = argv[index]!;
    if (value === "--") {
      operands.push(...argv.slice(index + 1).filter((item) => item !== "-"));
      break;
    }
    if (value === "-c" || value === "-n") {
      index += 1;
      continue;
    }
    if (value === "-" || value.startsWith("-")) continue;
    operands.push(value);
  }
  return operands;
}

function expandOperand(
  operand: string,
  cwd: string,
): { paths: string[]; pattern: boolean } {
  const pattern = /[*?[\]]/u.test(operand);
  if (!pattern) {
    return {
      paths: [isAbsolute(operand) ? resolve(operand) : resolve(cwd, operand)],
      pattern: false,
    };
  }
  return {
    paths: globSync(operand, { cwd }).map((path) => resolve(cwd, path)).sort(),
    pattern: true,
  };
}

function opaqueAtom(
  runtime: string,
  source: string,
  reason: string,
): OpaqueCodeCapability {
  return {
    kind: "opaque_code",
    runtime,
    digest: digestValue({ runtime, source }),
    reason,
  };
}
