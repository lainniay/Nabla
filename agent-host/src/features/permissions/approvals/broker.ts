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

export type ApprovalSelection =
  | { decision: "deny" }
  | { decision: ApprovalDecision; bundle: GrantBundle };

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
    proposals: GrantBundle[],
    requester: ApprovalRequester,
    signal?: AbortSignal,
  ): Promise<ApprovalSelection> {
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
    if (!selected || decision === "deny") return { decision: "deny" };
    return { decision, bundle: selected };
  }

  commit(
    requestId: string,
    intent: PermissionIntent,
    identity: WorkspaceIdentity,
    bundle: GrantBundle,
  ): void {
    if (bundle.scope === "once") {
      this.once.put({
        requestId,
        toolCallId: intent.toolCallId,
        intentDigest: intent.digest,
        sessionId: intent.sessionId,
        workspaceId: intent.workspaceId,
        bundle,
      });
    } else if (bundle.scope === "session") {
      this.session.add(bundle);
    } else {
      this.workspace.add(bundle, identity);
    }
  }
}
