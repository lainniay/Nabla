import type { GrantBundle, PermissionIntent } from "../model.ts";

export interface OnceGrant {
  requestId: string;
  toolCallId: string;
  intentDigest: string;
  sessionId: string;
  workspaceId: string;
  bundle: GrantBundle;
}

export class OnceGrantStore {
  private readonly grants = new Map<string, OnceGrant>();

  put(grant: OnceGrant): void {
    this.grants.set(grant.requestId, grant);
  }

  peek(intent: PermissionIntent, requestId: string): GrantBundle | undefined {
    const grant = this.grants.get(requestId);
    return grant && matches(grant, intent) ? grant.bundle : undefined;
  }

  consume(intent: PermissionIntent, requestId: string): GrantBundle | undefined {
    const grant = this.grants.get(requestId);
    if (!grant) return undefined;
    this.grants.delete(requestId);
    return matches(grant, intent) ? grant.bundle : undefined;
  }

  invalidate(requestId: string): void {
    this.grants.delete(requestId);
  }
}

function matches(grant: OnceGrant, intent: PermissionIntent): boolean {
  return (
    grant.toolCallId === intent.toolCallId &&
    grant.intentDigest === intent.digest &&
    grant.sessionId === intent.sessionId &&
    grant.workspaceId === intent.workspaceId
  );
}
