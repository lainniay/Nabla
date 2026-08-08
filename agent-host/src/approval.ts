import type {
  ApprovalDecision,
  GrantProposal,
} from "./protocol/schemas/permissions.ts";
import { RequestQueue } from "./features/interactions/request-queue.ts";

export type { ApprovalDecision } from "./protocol/schemas/permissions.ts";

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
  reason?: string;
}

export class ApprovalQueue {
  private readonly queue = new RequestQueue<undefined, ApprovalDecision>();

  request(
    request: ApprovalRequest,
    signal: AbortSignal | undefined,
    notify: (event: Record<string, unknown>) => void,
  ): Promise<ApprovalDecision> {
    return this.queue.request(
      request.requestId,
      undefined,
      signal ? [signal] : [],
      () =>
        notify({
          type: "approval_request",
          ...request,
        }),
      {
        onAbort: (pending) => pending.resolve("deny"),
        onNotifyError: (pending) => pending.resolve("deny"),
      },
    );
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
    return this.queue.reply(requestId, decision);
  }

  denyAll(): void {
    this.queue.settleAll((pending) => pending.resolve("deny"));
  }
}
