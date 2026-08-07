import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { SessionManager } from "@earendil-works/pi-coding-agent";

import {
  PLAN_ENTRY_TYPE,
  PLAN_MODE_ENTRY_TYPE,
  type PlanArtifact,
} from "./features/plans/model.ts";

const artifact: PlanArtifact = {
  id: "plan-1",
  revision: 2,
  title: "Structured planning",
  summary: "Treat plans as immutable artifacts.",
  bodyMarkdown: "Implement the artifact flow.",
  assumptions: ["Rust owns interaction"],
  testPlan: ["Run cargo test"],
  handoffMarkdown: "Preserve the artifact across sessions.",
  sourceSessionId: "session-1",
  createdAt: "2026-01-01T00:00:00.000Z",
  updatedAt: "2026-01-01T00:00:01.000Z",
};

test("fresh execution session contains the plan and inactive mode but no execution message", () => {
  const root = mkdtempSync(join(tmpdir(), "nabla-plan-session-"));
  try {
    const old = SessionManager.create(root, root);
    old.appendCustomMessageEntry("test.old-build", "old build context", false);
    const oldFile = old.getSessionFile();
    assert.ok(oldFile);

    const fresh = SessionManager.create(root, root, { parentSession: oldFile });
    fresh.appendCustomEntry(PLAN_ENTRY_TYPE, artifact);
    fresh.appendCustomEntry(PLAN_MODE_ENTRY_TYPE, { active: false });

    const entries = fresh.getBranch();
    const planEntry = entries.find(
      (entry) => entry.type === "custom" && entry.customType === PLAN_ENTRY_TYPE,
    );
    const modeEntry = entries.find(
      (entry) =>
        entry.type === "custom" && entry.customType === PLAN_MODE_ENTRY_TYPE,
    );
    assert.deepEqual((planEntry as { data?: unknown } | undefined)?.data, artifact);
    assert.deepEqual((modeEntry as { data?: unknown } | undefined)?.data, { active: false });
    assert.ok(!entries.some((entry) => entry.type === "message"));

    const context = fresh.buildSessionContext();
    const serialized = JSON.stringify(context.messages);
    assert.doesNotMatch(serialized, /old build context/u);
    assert.equal(fresh.getHeader()?.parentSession, oldFile);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
