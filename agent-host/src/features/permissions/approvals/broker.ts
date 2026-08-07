import type {
  ApprovalDecision,
  GrantBundle,
  PermissionIntent,
} from "../model.ts";
import type { WorkspaceIdentity } from "../workspace-identity.ts";
import { OnceGrantStore } from "./once-store.ts";
import { SessionGrantStore } from "./session-store.ts";
import { WorkspaceGrantStore } from "./workspace-store.ts";

export interface ApprovalPrompt {
  requestId: string;
  intent: PermissionIntent;
  proposals: GrantBundle[];
}

export type ApprovalRequester = (
  prompt: ApprovalPrompt,
  signal?: AbortSignal,
) => Promise<ApprovalDecision>;

export class ApprovalBroker {
  readonly once: OnceGrantStore;
  readonly session: SessionGrantStore;
  readonly workspace: WorkspaceGrantStore;

  constructor(
    once = new OnceGrantStore(),
    session = new SessionGrantStore(),
    workspace = new WorkspaceGrantStore(),
  ) {
    this.once = once;
    this.session = session;
    this.workspace = workspace;
  }

  grants(intent: PermissionIntent, identity: WorkspaceIdentity): GrantBundle[] {
    return [
      ...this.session.get(intent.sessionId, intent.workspaceId),
      ...this.workspace.get(identity),
    ];
  }

  async request(
    requestId: string,
    intent: PermissionIntent,
    identity: WorkspaceIdentity,
    proposals: GrantBundle[],
    requester: ApprovalRequester,
    signal?: AbortSignal,
  ): Promise<ApprovalDecision> {
    const decision = await requester({ requestId, intent, proposals }, signal);
    const selected = proposals.find((proposal) =>
      decision === "allow_once"
        ? proposal.scope === "once"
        : decision === "allow_session"
          ? proposal.scope === "session"
          : decision === "allow_workspace"
            ? proposal.scope === "workspace"
            : false,
    );
    if (!selected || decision === "deny") return "deny";
    if (selected.scope === "once") {
      this.once.put({
        requestId,
        toolCallId: intent.toolCallId,
        intentDigest: intent.digest,
        sessionId: intent.sessionId,
        workspaceId: intent.workspaceId,
        bundle: selected,
      });
    } else if (selected.scope === "session") {
      this.session.add(selected);
    } else {
      this.workspace.add(selected, identity);
    }
    return decision;
  }
}
