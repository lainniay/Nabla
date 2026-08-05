import { isAbsolute, relative, resolve, sep } from "node:path";

import type {
  CapabilityAtom,
  CapabilityMatcher,
  GrantBundle,
  PermissionIntent,
  PermissionRule,
  PolicyEffect,
} from "./model.ts";
import { digestValue } from "./shell/digest.ts";

export interface AtomEvaluation {
  atom: CapabilityAtom;
  effect: PolicyEffect;
  rules: PermissionRule[];
  grants: CapabilityMatcher[];
}

export interface PermissionEvaluation {
  effect: PolicyEffect;
  atoms: AtomEvaluation[];
}

const PRIORITY: Record<PolicyEffect, number> = {
  allow: 1,
  ask: 2,
  deny: 3,
};

export function evaluatePermission(
  intent: PermissionIntent,
  rules: readonly PermissionRule[],
  grants: readonly GrantBundle[] = [],
): PermissionEvaluation {
  const atoms = intent.atoms.map((atom): AtomEvaluation => {
    const matchingRules = rules.filter((rule) =>
      matcherMatches(rule.matcher, atom, intent),
    );
    const matchingGrants = grants
      .filter(
        (grant) =>
          grant.workspaceId === intent.workspaceId &&
          (grant.scope !== "session" || grant.sessionId === intent.sessionId),
      )
      .flatMap((grant) => grant.matchers)
      .filter((matcher) => matcherMatches(matcher, atom, intent));
    let effect: PolicyEffect =
      matchingRules.length === 0 && matchingGrants.length === 0 ? "ask" : "allow";
    for (const rule of matchingRules) {
      if (PRIORITY[rule.effect] > PRIORITY[effect]) effect = rule.effect;
    }
    return { atom, effect, rules: matchingRules, grants: matchingGrants };
  });
  const effect = atoms.reduce<PolicyEffect>(
    (current, atom) =>
      PRIORITY[atom.effect] > PRIORITY[current] ? atom.effect : current,
    intent.atoms.length === 0 ? "ask" : "allow",
  );
  return { effect, atoms };
}

export function matcherMatches(
  matcher: CapabilityMatcher,
  atom: CapabilityAtom,
  intent?: PermissionIntent,
): boolean {
  if (matcher.kind === "tool") {
    return (
      intent !== undefined &&
      matcher.tool === intent.tool &&
      (matcher.inputDigest === undefined ||
        matcher.inputDigest === digestValue(intent.normalizedInput))
    );
  }
  if (
    matcher.kind === "shell_intent" ||
    matcher.kind === "opaque_shell_exact"
  ) {
    if (
      !intent ||
      intent.tool !== "bash" ||
      typeof intent.normalizedInput !== "object" ||
      intent.normalizedInput === null
    ) {
      return false;
    }
    const command = (intent.normalizedInput as { command?: unknown }).command;
    return (
      typeof command === "string" &&
      normalizeCommand(command) === normalizeCommand(matcher.command)
    );
  }
  if (matcher.kind !== atom.kind) return false;
  switch (matcher.kind) {
    case "exec":
      return (
        atom.kind === "exec" &&
        matcher.executable === atom.executable &&
        (matcher.argv === undefined || arraysEqual(matcher.argv, atom.argv)) &&
        (matcher.cwd === undefined || resolve(matcher.cwd) === resolve(atom.cwd)) &&
        (matcher.environment === undefined ||
          recordsEqual(matcher.environment, atom.environment))
      );
    case "file":
      return (
        atom.kind === "file" &&
        matcher.operation === atom.operation &&
        pathMatches(matcher.path, atom.path, matcher.recursive === true) &&
        (matcher.destination === undefined ||
          (atom.destination !== undefined &&
            pathMatches(matcher.destination, atom.destination, false)))
      );
    case "network":
      return (
        atom.kind === "network" &&
        matcher.operation === atom.operation &&
        matcher.host === atom.host &&
        (matcher.port === undefined || matcher.port === atom.port) &&
        (matcher.protocol === undefined || matcher.protocol === atom.protocol)
      );
    case "opaque_code":
      return (
        atom.kind === "opaque_code" &&
        matcher.runtime === atom.runtime &&
        matcher.digest === atom.digest
      );
  }
}

function normalizeCommand(command: string): string {
  return command.trim().replace(/\s+/gu, " ");
}

function pathMatches(base: string, candidate: string, recursive: boolean): boolean {
  const normalizedBase = resolve(base);
  const normalizedCandidate = resolve(candidate);
  if (normalizedBase === normalizedCandidate) return true;
  if (!recursive) return false;
  const child = relative(normalizedBase, normalizedCandidate);
  return child !== "" && child !== ".." && !child.startsWith(`..${sep}`) &&
    !isAbsolute(child);
}

function arraysEqual(left: readonly string[], right: readonly string[]): boolean {
  return left.length === right.length &&
    left.every((value, index) => value === right[index]);
}

function recordsEqual(
  left: Record<string, string>,
  right: Record<string, string>,
): boolean {
  const leftKeys = Object.keys(left).sort();
  const rightKeys = Object.keys(right).sort();
  return arraysEqual(leftKeys, rightKeys) &&
    leftKeys.every((key) => left[key] === right[key]);
}
