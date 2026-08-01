export type ApprovalDecision = "allow" | "allow_goal" | "deny";

export interface ApprovalRequest {
  toolCallId: string;
  toolName: string;
  input: unknown;
  agentId?: string;
  agentProfile?: string;
  model?: string;
  goalId?: string;
  reason?: string;
  risk?: "normal" | "elevated" | "high" | "credential" | "outside_workspace";
}

interface PendingApproval {
  resolve(decision: ApprovalDecision): void;
}

export class ApprovalQueue {
  private readonly pending = new PendingRequestRegistry<PendingApproval>();
  private nextId = 1;

  request(
    request: ApprovalRequest,
    signal: AbortSignal | undefined,
    notify: (event: Record<string, unknown>) => void,
  ): Promise<ApprovalDecision> {
    const approvalId = `approval-${this.nextId++}`;
    return new Promise<ApprovalDecision>((resolveDecision) => {
      const onAbort = () => {
        this.pending.take(approvalId)?.resolve("deny");
      };
      this.pending.register(
        approvalId,
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
          approvalId,
          ...request,
        });
      } catch {
        this.pending.take(approvalId)?.resolve("deny");
      }
    });
  }

  reply(approvalId: string, decision: ApprovalDecision): boolean {
    const approval = this.pending.take(approvalId);
    if (!approval) return false;
    approval.resolve(decision);
    return true;
  }

  denyAll(): void {
    for (const approval of this.pending.drain()) approval.resolve("deny");
  }
}
import { PendingRequestRegistry } from "./protocol/pending-request-registry.ts";
