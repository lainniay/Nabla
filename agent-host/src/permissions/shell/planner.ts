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

export interface ExecutionPlan {
  source: string;
  atoms: CapabilityAtom[];
  commands: ExecCapability[];
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
  let currentCwd = resolve(cwd);
  let opaque = false;
  let requiresShell =
    script.connectors.length > 0 ||
    script.nodes.some((node) => node.type === "group");

  if (script.opaqueReason) {
    atoms.push(opaqueAtom("shell", script.source, script.opaqueReason));
    return {
      source: script.source,
      atoms,
      commands,
      opaque: true,
      requiresShell: true,
    };
  }

  for (const node of script.nodes) {
    if (node.type === "group") {
      const nested = planParsedShell(node.script, currentCwd, environment);
      atoms.push(...nested.atoms);
      commands.push(...nested.commands);
      opaque ||= nested.opaque;
      requiresShell ||= nested.requiresShell;
      continue;
    }
    const planned = planCommand(node, currentCwd, environment);
    atoms.push(...planned.atoms);
    commands.push(...planned.commands);
    opaque ||= planned.opaque;
    requiresShell ||= node.redirections.length > 0 ||
      node.substitutions.length > 0;
    if (node.argv[0] === "cd" && node.argv.length === 2 && !node.opaqueReason) {
      currentCwd = resolve(currentCwd, node.argv[1]!);
    }
  }
  return { source: script.source, atoms, commands, opaque, requiresShell };
}

function planCommand(
  command: ShellCommand,
  cwd: string,
  inheritedEnvironment: Record<string, string>,
): Pick<ExecutionPlan, "atoms" | "commands" | "opaque"> {
  const atoms: CapabilityAtom[] = [];
  const commands: ExecCapability[] = [];
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
    const exec: ExecCapability = {
      kind: "exec",
      executable: command.argv[0]!,
      argv: command.argv.slice(1),
      cwd,
      environment: { ...inheritedEnvironment, ...command.assignments },
    };
    atoms.push(exec);
    commands.push(exec);

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
    opaque ||= nested.opaque;
  }
  return { atoms, commands, opaque };
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
