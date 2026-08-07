import { appendFileSync, mkdirSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";

import type {
  ApprovalDecision,
  PermissionEvaluation,
} from "./kernel.ts";
import type { PermissionIntent } from "./model.ts";
import type { SandboxExecutionProfile } from "./execution/sandbox-profile.ts";
import { digestValue } from "./shell/digest.ts";

export interface PermissionAuditEntry {
  timestamp: string;
  requestId: string;
  intentId: string;
  toolCallId: string;
  sessionId: string;
  workspaceId: string;
  intentDigest: string;
  capabilityAtoms: unknown[];
  risk: "normal" | "elevated" | "high" | "credential" | "outside_workspace";
  effect: "allow" | "ask" | "deny";
  decision?: ApprovalDecision;
  grantScope?: "once" | "session" | "workspace";
  sandboxProfile?: SandboxExecutionProfile;
  onceConsumed?: boolean;
  outcome:
    | "automatic_allow"
    | "denied"
    | "authorized"
    | "preflight_rejected"
    | "execution_started"
    | "executed"
    | "execution_failed";
  matchedRules: Array<{
    id: string;
    source: string;
    effect: "allow" | "ask" | "deny";
  }>;
  matchedGrantDigests: string[];
  matchedGrants: Array<{
    digest: string;
    scope: "once" | "session" | "workspace";
    workspaceId: string;
    sessionId?: string;
  }>;
}

export interface PermissionAuditSink {
  record(entry: PermissionAuditEntry): void;
}

export class JsonlPermissionAuditLog implements PermissionAuditSink {
  private readonly path: string;

  constructor(homeDir = homedir()) {
    this.path = join(homeDir, ".nabla", "permission-audit.jsonl");
  }

  record(entry: PermissionAuditEntry): void {
    mkdirSync(dirname(this.path), { recursive: true });
    appendFileSync(this.path, `${JSON.stringify(entry)}\n`, { mode: 0o600 });
  }
}

export class MemoryPermissionAuditLog implements PermissionAuditSink {
  readonly entries: PermissionAuditEntry[] = [];

  record(entry: PermissionAuditEntry): void {
    this.entries.push(entry);
  }
}

export function auditEntry(
  requestId: string,
  intent: PermissionIntent,
  evaluation: PermissionEvaluation,
  options: {
    decision?: ApprovalDecision;
    risk?: PermissionAuditEntry["risk"];
    sandboxProfile?: SandboxExecutionProfile;
    onceConsumed?: boolean;
    outcome?: PermissionAuditEntry["outcome"];
  } = {},
): PermissionAuditEntry {
  const grantScope = options.decision === "allow_once"
    ? "once"
    : options.decision === "allow_session"
      ? "session"
      : options.decision === "allow_workspace"
        ? "workspace"
        : undefined;
  return {
    timestamp: new Date().toISOString(),
    requestId,
    intentId: intent.id,
    toolCallId: intent.toolCallId,
    sessionId: intent.sessionId,
    workspaceId: intent.workspaceId,
    intentDigest: intent.digest,
    capabilityAtoms: intent.atoms.map(redactedAtom),
    risk: options.risk ?? "normal",
    effect: evaluation.effect,
    ...(options.decision ? { decision: options.decision } : {}),
    ...(grantScope ? { grantScope } : {}),
    ...(options.sandboxProfile
      ? { sandboxProfile: options.sandboxProfile }
      : {}),
    ...(options.onceConsumed === undefined
      ? {}
      : { onceConsumed: options.onceConsumed }),
    outcome: options.outcome ??
      (evaluation.effect === "deny"
        ? "denied"
        : evaluation.effect === "allow" && !options.decision
          ? "automatic_allow"
          : "authorized"),
    matchedRules: [
      ...new Map(
        evaluation.atoms.flatMap((atom) =>
          atom.rules.map((rule) => [
            rule.id,
            { id: rule.id, source: rule.source, effect: rule.effect },
          ] as const)),
      ).values(),
    ],
    matchedGrantDigests: [
      ...new Set(
        evaluation.atoms.flatMap((atom) =>
          atom.grants.map((grant) => digestValue(grant.matcher))),
      ),
    ],
    matchedGrants: [
      ...new Map(
        evaluation.atoms.flatMap((atom) =>
          atom.grants.map((grant) => {
            const digest = digestValue(grant.matcher);
            return [
              `${grant.scope}:${grant.workspaceId}:${grant.sessionId ?? ""}:${digest}`,
              {
                digest,
                scope: grant.scope,
                workspaceId: grant.workspaceId,
                ...(grant.sessionId ? { sessionId: grant.sessionId } : {}),
              },
            ] as const;
          })),
      ).values(),
    ],
  };
}

function redactedAtom(atom: PermissionIntent["atoms"][number]): unknown {
  if (atom.kind === "exec") {
    return {
      kind: atom.kind,
      executable: atom.executable,
      argvDigest: digestValue(atom.argv),
      cwd: atom.cwd,
      environmentKeys: Object.keys(atom.environment).sort(),
      environmentDigest: digestValue(atom.environment),
    };
  }
  return atom;
}
