import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { SessionManager } from "@earendil-works/pi-coding-agent";

import { PLAN_ENTRY_TYPE, PLAN_EXECUTION_MESSAGE_TYPE } from "./plan.ts";

test("fresh execution session contains the plan but excludes old build messages", () => {
  const root = mkdtempSync(join(tmpdir(), "nabla-plan-session-"));
  try {
    const old = SessionManager.create(root, root);
    old.appendCustomMessageEntry("test.old-build", "old build context", false);
    const oldFile = old.getSessionFile();
    assert.ok(oldFile);

    const fresh = SessionManager.create(root, root, { parentSession: oldFile });
    fresh.appendCustomEntry(PLAN_ENTRY_TYPE, {
      schemaVersion: 1,
      id: "plan-1",
      revision: 1,
      status: "executing",
    });
    fresh.appendCustomMessageEntry(
      PLAN_EXECUTION_MESSAGE_TYPE,
      "execute exact plan artifact",
      false,
      { planId: "plan-1", revision: 1 },
    );

    const context = fresh.buildSessionContext();
    const serialized = JSON.stringify(context.messages);
    assert.match(serialized, /execute exact plan artifact/);
    assert.doesNotMatch(serialized, /old build context/);
    assert.equal(fresh.getHeader()?.parentSession, oldFile);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
