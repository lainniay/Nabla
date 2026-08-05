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
import type { ApprovalDecision, PermissionIntent } from "./model.ts";
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
  ): Promise<Authorization> {
    let evaluation = evaluatePermission(
      intent,
      [...this.policies.all(), ...additionalRules],
      this.approvals.grants(intent, identity),
    );
    if (evaluation.effect === "deny" || evaluation.effect === "allow") {
      this.audit.record(auditEntry(intent, evaluation));
      return { requestId, intent, evaluation };
    }
    const proposals = proposeGrantBundles(intent, identity).filter(
      (bundle) => allowWorkspaceGrant || bundle.scope !== "workspace",
    );
    const decision = await this.approvals.request(
      requestId,
      intent,
      proposals,
      requester,
      signal,
    );
    if (decision === "deny") {
      this.audit.record(auditEntry(intent, evaluation, decision));
      return { requestId, intent, evaluation, decision };
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
    this.audit.record(auditEntry(intent, evaluation, decision));
    return { requestId, intent, evaluation, decision };
  }

  consumeForExecution(
    authorization: Authorization,
    recomputedIntent: PermissionIntent,
  ): boolean {
    if (
      authorization.intent.digest !== recomputedIntent.digest ||
      authorization.intent.toolCallId !== recomputedIntent.toolCallId ||
      authorization.intent.sessionId !== recomputedIntent.sessionId ||
      authorization.intent.workspaceId !== recomputedIntent.workspaceId
    ) {
      if (authorization.decision === "allow_once") {
        this.approvals.once.consume(
          recomputedIntent,
          authorization.requestId,
        );
      }
      return false;
    }
    if (authorization.decision === "allow_once") {
      return this.approvals.once.consume(
        recomputedIntent,
        authorization.requestId,
      ) !== undefined;
    }
    if (
      authorization.decision === "allow_session" ||
      authorization.decision === "allow_workspace"
    ) {
      return true;
    }
    return authorization.evaluation.effect === "allow";
  }
}
