import { auditEntry, type PermissionAuditSink } from "./audit-log.ts";
import {
  ApprovalBroker,
  type ApprovalRequester,
} from "./approvals/broker.ts";
import {
  evaluatePermission,
  type PermissionEvaluation,
} from "./evaluator.ts";
import type { SandboxExecutionProfile } from "./execution/sandbox-profile.ts";
import { proposeGrantBundles } from "./grant-proposal.ts";
import type {
  ApprovalDecision,
  PermissionIntent,
} from "./model.ts";
import type { PermissionRule } from "./model.ts";
import { PolicyStore } from "./policy-store.ts";
import type { WorkspaceIdentity } from "./workspace-identity.ts";

export type { ApprovalDecision } from "./model.ts";
export type { PermissionEvaluation } from "./evaluator.ts";

export interface Authorization {
  requestId: string;
  intent: PermissionIntent;
  evaluation: PermissionEvaluation;
  decision?: ApprovalDecision;
  risk: "normal" | "elevated" | "high" | "credential" | "outside_workspace";
  identity: WorkspaceIdentity;
  additionalRules: PermissionRule[];
  policyRevisionAtPrompt: number;
  deniedReason?: "policy_changed";
}

export class PermissionKernel {
  readonly policies: PolicyStore;
  readonly approvals: ApprovalBroker;
  readonly audit: PermissionAuditSink;

  constructor(
    policies: PolicyStore,
    approvals: ApprovalBroker,
    audit: PermissionAuditSink,
  ) {
    this.policies = policies;
    this.approvals = approvals;
    this.audit = audit;
  }

  async authorize(
    requestId: string,
    intent: PermissionIntent,
    identity: WorkspaceIdentity,
    requester: ApprovalRequester,
    signal?: AbortSignal,
    additionalRules: readonly PermissionRule[] = [],
    allowWorkspaceGrant = true,
    risk: Authorization["risk"] = "normal",
  ): Promise<Authorization> {
    const rules = [...this.policies.all(), ...additionalRules];
    let evaluation = evaluatePermission(
      intent,
      rules,
      this.approvals.grants(intent, identity),
    );
    if (evaluation.effect === "deny" || evaluation.effect === "allow") {
      this.audit.record(auditEntry(requestId, intent, evaluation, { risk }));
      return {
        requestId,
        intent,
        evaluation,
        risk,
        identity,
        additionalRules: [...additionalRules],
        policyRevisionAtPrompt: this.policies.revision,
      };
    }
    const unresolvedAtoms = evaluation.atoms
      .filter((atom) => atom.effect !== "allow")
      .map((atom) => atom.atom);
    const proposals = proposeGrantBundles(intent, identity, unresolvedAtoms).filter(
      (bundle) => allowWorkspaceGrant || bundle.scope !== "workspace",
    );
    const policyRevisionAtPrompt = this.policies.revision;
    const selection = await this.approvals.request(
      requestId,
      intent,
      proposals,
      requester,
      signal,
    );
    if (selection.decision === "deny") {
      this.audit.record(auditEntry(requestId, intent, evaluation, {
        decision: "deny",
        risk,
        outcome: "denied",
      }));
      return {
        requestId,
        intent,
        evaluation,
        decision: "deny",
        risk,
        identity,
        additionalRules: [...additionalRules],
        policyRevisionAtPrompt,
      };
    }
    evaluation = evaluatePermission(
      intent,
      [...this.policies.all(), ...additionalRules],
      [
        ...this.approvals.grants(intent, identity),
        selection.bundle,
      ],
    );
    if (
      this.policies.revision !== policyRevisionAtPrompt &&
      evaluation.effect !== "allow"
    ) {
      this.audit.record(auditEntry(requestId, intent, evaluation, {
        decision: "deny",
        risk,
        outcome: "denied",
      }));
      return {
        requestId,
        intent,
        evaluation,
        decision: "deny",
        risk,
        identity,
        additionalRules: [...additionalRules],
        policyRevisionAtPrompt,
        deniedReason: "policy_changed",
      };
    }
    this.approvals.commit(requestId, intent, identity, selection.bundle);
    const once = this.approvals.once.peek(intent, requestId);
    evaluation = evaluatePermission(
      intent,
      [...this.policies.all(), ...additionalRules],
      [
        ...this.approvals.grants(intent, identity),
        ...(once ? [once] : []),
      ],
    );
    this.audit.record(auditEntry(requestId, intent, evaluation, {
      decision: selection.decision,
      risk,
      outcome: "authorized",
    }));
    return {
      requestId,
      intent,
      evaluation,
      decision: selection.decision,
      risk,
      identity,
      additionalRules: [...additionalRules],
      policyRevisionAtPrompt,
    };
  }

  consume(
    authorization: Authorization,
    recomputedIntent: PermissionIntent,
    sandboxProfile?: SandboxExecutionProfile | null,
  ): boolean {
    if (
      authorization.intent.digest !== recomputedIntent.digest ||
      authorization.intent.toolCallId !== recomputedIntent.toolCallId ||
      authorization.intent.sessionId !== recomputedIntent.sessionId ||
      authorization.intent.workspaceId !== recomputedIntent.workspaceId
    ) {
      if (authorization.decision === "allow_once") {
        this.approvals.once.invalidate(authorization.requestId);
      }
      this.audit.record(auditEntry(
        authorization.requestId,
        recomputedIntent,
        authorization.evaluation,
        {
          decision: authorization.decision,
          risk: authorization.risk,
          sandboxProfile: sandboxProfile ?? undefined,
          onceConsumed: false,
          outcome: "preflight_rejected",
        },
      ));
      return false;
    }
    const onceGrant = authorization.decision === "allow_once"
      ? this.approvals.once.peek(recomputedIntent, authorization.requestId)
      : undefined;
    const evaluation = evaluatePermission(
      recomputedIntent,
      [...this.policies.all(), ...authorization.additionalRules],
      [
        ...this.approvals.grants(recomputedIntent, authorization.identity),
        ...(onceGrant ? [onceGrant] : []),
      ],
    );
    const revisionUnchanged =
      this.policies.revision === authorization.policyRevisionAtPrompt;
    let authorized: boolean;
    let onceConsumed: boolean | undefined;
    if (authorization.decision === "allow_once") {
      authorized = onceGrant !== undefined &&
        (revisionUnchanged || evaluation.effect === "allow");
      if (authorized) {
        this.approvals.once.consume(recomputedIntent, authorization.requestId);
        onceConsumed = true;
      } else {
        this.approvals.once.invalidate(authorization.requestId);
      }
    } else if (
      authorization.decision === "allow_session" ||
      authorization.decision === "allow_workspace"
    ) {
      const scope =
        authorization.decision === "allow_session" ? "session" : "workspace";
      const grantStillMatches = evaluation.atoms.some((atom) =>
        atom.grants.some((grant) => grant.scope === scope),
      );
      authorized =
        evaluation.effect === "allow" ||
        (revisionUnchanged && grantStillMatches);
    } else {
      authorized =
        authorization.decision !== "deny" && evaluation.effect === "allow";
    }
    this.audit.record(auditEntry(
      authorization.requestId,
      recomputedIntent,
      evaluation,
      {
        decision: authorization.decision,
        risk: authorization.risk,
        sandboxProfile: sandboxProfile ?? undefined,
        onceConsumed,
        outcome: authorized ? "execution_started" : "preflight_rejected",
      },
    ));
    return authorized;
  }

  recordResult(
    authorization: Authorization,
    sandboxProfile: SandboxExecutionProfile | null,
    succeeded: boolean,
  ): void {
    this.audit.record(auditEntry(
      authorization.requestId,
      authorization.intent,
      authorization.evaluation,
      {
        decision: authorization.decision,
        risk: authorization.risk,
        sandboxProfile: sandboxProfile ?? undefined,
        outcome: succeeded ? "executed" : "execution_failed",
      },
    ));
  }
}
