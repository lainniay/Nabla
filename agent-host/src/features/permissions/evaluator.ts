import { resolve } from "node:path";

import type {
  CapabilityAtom,
  CapabilityMatcher,
  FileCapabilityMatcher,
  GrantBundle,
  PermissionIntent,
  PermissionRule,
  PolicyEffect,
} from "./model.ts";
import { digestValue } from "./shell/digest.ts";
import { isPathWithin, patternMatches } from "./filesystem/path.ts";

export interface AtomEvaluation {
  atom: CapabilityAtom;
  effect: PolicyEffect;
  rules: PermissionRule[];
  grants: Array<{
    scope: GrantBundle["scope"];
    workspaceId: string;
    sessionId?: string;
    matcher: CapabilityMatcher;
  }>;
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
      .flatMap((grant) =>
        grant.matchers
          .filter((matcher) => matcherMatches(matcher, atom, intent))
          .map((matcher) => ({
            scope: grant.scope,
            workspaceId: grant.workspaceId,
            ...(grant.sessionId ? { sessionId: grant.sessionId } : {}),
            matcher,
          })));
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
  // Legacy shell_digest grants bind only the command string, so they can
  // authorize the same command against different cwd/files. They are retired:
  // stored grants stay in documents but never match.
  if (matcher.kind === "shell_digest") return false;
  if (matcher.kind === "shell_command") {
    if (
      !intent ||
      intent.tool !== "bash" ||
      typeof intent.normalizedInput !== "object" ||
      intent.normalizedInput === null
    ) {
      return false;
    }
    const command = (intent.normalizedInput as { command?: unknown }).command;
    if (typeof command !== "string") return false;
    return patternMatches(
      matcher.pattern,
      command.trim().replace(/\s+/gu, " "),
    );
  }
  if (matcher.kind !== atom.kind) return false;
  switch (matcher.kind) {
    case "exec":
      return (
        atom.kind === "exec" &&
        (matcher.executable === "*" || matcher.executable === atom.executable) &&
        (matcher.argv === undefined || arraysEqual(matcher.argv, atom.argv)) &&
        (matcher.cwd === undefined || resolve(matcher.cwd) === resolve(atom.cwd)) &&
        (matcher.environment === undefined ||
          recordsEqual(matcher.environment, atom.environment))
      );
    case "file":
      return (
        atom.kind === "file" &&
        (matcher.operation === "*" || matcher.operation === atom.operation) &&
        pathMatches(matcher, atom.path) &&
        (matcher.destination === undefined ||
          (atom.destination !== undefined &&
            pathMatches(
              {
                ...matcher,
                path: matcher.destination,
                recursive: false,
                pattern: undefined,
              },
              atom.destination,
            )))
      );
    case "network":
      return (
        atom.kind === "network" &&
        matcher.operation === atom.operation &&
        (matcher.host === "*" || matcher.host === atom.host) &&
        (matcher.port === undefined || matcher.port === atom.port) &&
        (matcher.protocol === undefined || matcher.protocol === atom.protocol)
      );
    case "opaque_code":
      return (
        atom.kind === "opaque_code" &&
        (matcher.runtime === "*" || matcher.runtime === atom.runtime) &&
        matcher.digest === atom.digest
      );
  }
}

function pathMatches(
  matcher: FileCapabilityMatcher,
  candidate: string,
): boolean {
  if (matcher.path === "*") return true;
  if (matcher.pattern === true) return patternMatches(matcher.path, candidate);
  if (!isPathWithin(matcher.path, candidate)) return false;
  return matcher.recursive === true || resolve(matcher.path) === resolve(candidate);
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
