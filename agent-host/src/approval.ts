import type { GrantProposal } from "./permissions/model.ts";
import { PendingRequestRegistry } from "./protocol/pending-request-registry.ts";

export type ApprovalDecision =
  | "allow_once"
  | "allow_session"
  | "allow_workspace"
  | "deny";

export interface ApprovalRequest {
  requestId: string;
  toolCallId: string;
  sessionId: string;
  workspaceId: string;
  summary: string;
  risk: "normal" | "elevated" | "high" | "credential" | "outside_workspace";
  intentDigest: string;
  availableDecisions: ApprovalDecision[];
  sessionGrant?: GrantProposal;
  workspaceGrant?: GrantProposal;
  toolName: string;
  input: unknown;
  agentId?: string;
  agentProfile?: string;
  model?: string;
  goalId?: string;
  reason?: string;
}

interface PendingApproval {
  resolve(decision: ApprovalDecision): void;
}

export class ApprovalQueue {
  private readonly pending = new PendingRequestRegistry<PendingApproval>();

  request(
    request: ApprovalRequest,
    signal: AbortSignal | undefined,
    notify: (event: Record<string, unknown>) => void,
  ): Promise<ApprovalDecision> {
    return new Promise<ApprovalDecision>((resolveDecision) => {
      const onAbort = () => {
        this.pending.take(request.requestId)?.resolve("deny");
      };
      this.pending.register(
        request.requestId,
        { resolve: resolveDecision },
        () => signal?.removeEventListener("abort", onAbort),
      );
      signal?.addEventListener("abort", onAbort, { once: true });
      if (signal?.aborted) {
        onAbort();
        return;
      }
      try {
        notify({
          type: "approval_request",
          ...request,
        });
      } catch {
        this.pending.take(request.requestId)?.resolve("deny");
      }
    });
  }

  reply(requestId: string, decision: ApprovalDecision): boolean {
    if (
      decision !== "allow_once" &&
      decision !== "allow_session" &&
      decision !== "allow_workspace" &&
      decision !== "deny"
    ) {
      throw new Error(`Unsupported approval decision: ${String(decision)}`);
    }
    const approval = this.pending.take(requestId);
    if (!approval) return false;
    approval.resolve(decision);
    return true;
  }

  denyAll(): void {
    for (const approval of this.pending.drain()) approval.resolve("deny");
  }
}
