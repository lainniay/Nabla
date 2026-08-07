import { auditEntry, type PermissionAuditSink } from "./audit-log.ts";
import {
  ApprovalBroker,
  type ApprovalRequester,
} from "./approvals/broker.ts";
import {
  evaluatePermission,
  type PermissionEvaluation,
} from "./evaluator.ts";
import { proposeGrantBundles } from "./grant-proposal.ts";
import type {
  ApprovalDecision,
  ExecutionProfile,
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
    let evaluation = evaluatePermission(
      intent,
      [...this.policies.all(), ...additionalRules],
      this.approvals.grants(intent, identity),
    );
    if (evaluation.effect === "deny" || evaluation.effect === "allow") {
      this.audit.record(auditEntry(requestId, intent, evaluation, { risk }));
      return { requestId, intent, evaluation, risk };
    }
    const proposals = proposeGrantBundles(intent, identity).filter(
      (bundle) => allowWorkspaceGrant || bundle.scope !== "workspace",
    );
    const decision = await this.approvals.request(
      requestId,
      intent,
      identity,
      proposals,
      requester,
      signal,
    );
    if (decision === "deny") {
      this.audit.record(auditEntry(requestId, intent, evaluation, {
        decision,
        risk,
        outcome: "denied",
      }));
      return { requestId, intent, evaluation, decision, risk };
    }
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
      decision,
      risk,
      outcome: "authorized",
    }));
    return { requestId, intent, evaluation, decision, risk };
  }

  consumeForExecution(
    authorization: Authorization,
    recomputedIntent: PermissionIntent,
    executionProfile?: ExecutionProfile,
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
          executionProfile,
          onceConsumed: false,
          outcome: "preflight_rejected",
        },
      ));
      return false;
    }
    let authorized: boolean;
    let onceConsumed: boolean | undefined;
    if (authorization.decision === "allow_once") {
      authorized = this.approvals.once.consume(
        recomputedIntent,
        authorization.requestId,
      ) !== undefined;
      onceConsumed = authorized;
    } else if (
      authorization.decision === "allow_session" ||
      authorization.decision === "allow_workspace"
    ) {
      authorized = true;
    } else {
      authorized = authorization.evaluation.effect === "allow";
    }
    this.audit.record(auditEntry(
      authorization.requestId,
      recomputedIntent,
      authorization.evaluation,
      {
        decision: authorization.decision,
        risk: authorization.risk,
        executionProfile,
        onceConsumed,
        outcome: authorized ? "execution_started" : "preflight_rejected",
      },
    ));
    return authorized;
  }

  recordExecutionResult(
    authorization: Authorization,
    executionProfile: ExecutionProfile,
    succeeded: boolean,
  ): void {
    this.audit.record(auditEntry(
      authorization.requestId,
      authorization.intent,
      authorization.evaluation,
      {
        decision: authorization.decision,
        risk: authorization.risk,
        executionProfile,
        outcome: succeeded ? "executed" : "execution_failed",
      },
    ));
  }
}
