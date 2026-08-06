import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { SessionManager } from "@earendil-works/pi-coding-agent";

import type { RuntimeAccess } from "../../runtime/runtime-access.ts";
import type { JsonObject } from "../../protocol/validation.ts";
import { SessionBrowserService } from "./session-browser-service.ts";

test("open, query, close, and invalidate manage the catalog lifecycle", async () => {
  const root = mkdtempSync(join(tmpdir(), "nabla-session-browser-"));
  try {
    const manager = SessionManager.create(root, root);
    const runtime = {
      requireIdle: () => ({ session: { sessionManager: manager } }) as never,
      current: () => ({ session: { sessionManager: manager } }) as never,
      sessionGeneration: () => 1,
    } as unknown as RuntimeAccess;
    const events: JsonObject[] = [];
    const service = new SessionBrowserService(runtime, (event) =>
      events.push(event),
    );

    const opened = await service.open();
    assert.ok(opened.browserId);
    assert.equal(events.length, 0);

    const queried = await service.query({
      browserId: opened.browserId,
      scope: "current",
      sortMode: "threaded",
      query: "",
      namedOnly: false,
      offset: 0,
    });
    assert.equal(queried.browserId, opened.browserId);

    service.close(opened.browserId);
    await assert.rejects(
      service.query({
        browserId: opened.browserId,
        scope: "current",
        sortMode: "threaded",
        query: "",
        namedOnly: false,
        offset: 0,
      }),
      /Session browser is no longer active/u,
    );

    const reopened = await service.open();
    service.closeAll();
    await assert.rejects(
      service.query({
        browserId: reopened.browserId,
        scope: "current",
        sortMode: "threaded",
        query: "",
        namedOnly: false,
        offset: 0,
      }),
      /Session browser is no longer active/u,
    );
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
