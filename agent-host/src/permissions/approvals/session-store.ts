import type { GrantBundle } from "../model.ts";

export class SessionGrantStore {
  private readonly grants = new Map<string, GrantBundle[]>();

  add(bundle: GrantBundle): void {
    if (bundle.scope !== "session" || !bundle.sessionId) {
      throw new Error("Session grants require a sessionId and session scope");
    }
    const current = this.grants.get(bundle.sessionId) ?? [];
    current.push(bundle);
    this.grants.set(bundle.sessionId, current);
  }

  get(sessionId: string, workspaceId: string): GrantBundle[] {
    return (this.grants.get(sessionId) ?? []).filter(
      (bundle) => bundle.workspaceId === workspaceId,
    );
  }

  clear(sessionId: string): void {
    this.grants.delete(sessionId);
  }
}
