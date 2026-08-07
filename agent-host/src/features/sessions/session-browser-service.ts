import type { RuntimeAccess } from "../../runtime/runtime-access.ts";
import {
  SessionCatalog,
  type SessionBrowserSnapshot,
} from "./catalog.ts";
import type { JsonObject } from "../../protocol/validation.ts";

export class SessionBrowserService {
  private readonly catalogs = new Map<string, SessionCatalog>();
  private readonly runtime: RuntimeAccess;
  private readonly send: (event: JsonObject) => void;

  constructor(
    runtime: RuntimeAccess,
    send: (event: JsonObject) => void,
  ) {
    this.runtime = runtime;
    this.send = send;
  }

  async open(): Promise<SessionBrowserSnapshot> {
    const runtime = this.runtime.requireIdle("Cannot browse sessions");
    const catalog = new SessionCatalog({
      manager: runtime.session.sessionManager,
      onProgress: (browserId, scope, loaded, total) =>
        this.send({
          type: "session_list_progress",
          browserId,
          scope,
          loaded,
          total,
        }),
    });
    this.catalogs.set(catalog.browserId, catalog);
    return catalog.query("current", "", "threaded", false);
  }

  async query(input: {
    browserId: string;
    scope: "current" | "all";
    sortMode: "threaded" | "recent" | "relevance";
    query: string;
    namedOnly: boolean;
    offset: number;
  }): Promise<SessionBrowserSnapshot> {
    const catalog = this.catalogs.get(input.browserId);
    if (!catalog) throw new Error("Session browser is no longer active");
    return catalog.query(
      input.scope,
      input.query,
      input.sortMode,
      input.namedOnly,
      input.offset,
    );
  }

  close(browserId: string): void {
    this.catalogs.delete(browserId);
  }

  closeAll(): void {
    this.catalogs.clear();
  }
}
