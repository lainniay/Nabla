import { appendFileSync, mkdirSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";

import type {
  ApprovalDecision,
  PermissionEvaluation,
} from "./kernel.ts";
import type { PermissionIntent } from "./model.ts";
import { digestValue } from "./shell/digest.ts";

export interface PermissionAuditEntry {
  timestamp: string;
  intentId: string;
  toolCallId: string;
  sessionId: string;
  workspaceId: string;
  digest: string;
  effect: "allow" | "ask" | "deny";
  decision?: ApprovalDecision;
  matchedRuleIds: string[];
  matchedGrantDigests: string[];
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
  intent: PermissionIntent,
  evaluation: PermissionEvaluation,
  decision?: ApprovalDecision,
): PermissionAuditEntry {
  return {
    timestamp: new Date().toISOString(),
    intentId: intent.id,
    toolCallId: intent.toolCallId,
    sessionId: intent.sessionId,
    workspaceId: intent.workspaceId,
    digest: intent.digest,
    effect: evaluation.effect,
    ...(decision ? { decision } : {}),
    matchedRuleIds: [
      ...new Set(evaluation.atoms.flatMap((atom) => atom.rules.map((rule) => rule.id))),
    ],
    matchedGrantDigests: [
      ...new Set(
        evaluation.atoms.flatMap((atom) =>
          atom.grants.map((grant) => digestValue(grant))),
      ),
    ],
  };
}
