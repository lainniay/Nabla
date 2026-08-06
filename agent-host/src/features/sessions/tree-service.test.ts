import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { SessionManager } from "@earendil-works/pi-coding-agent";

import { ContextBudgetManager } from "../../context-manager.ts";
import { PlanStore } from "../../plan.ts";
import type { RuntimeAccess } from "../../runtime/runtime-access.ts";
import { PlanModeService } from "../../runtime/plan-mode-service.ts";
import type { JsonObject } from "../../protocol/validation.ts";
import { TreeService } from "./tree-service.ts";

test("state, label, and abort delegate to the session", async () => {
  const root = mkdtempSync(join(tmpdir(), "nabla-tree-service-"));
  try {
    const manager = SessionManager.create(root, root);
    let aborted = false;
    const session = {
      isIdle: true,
      sessionManager: manager,
      getActiveToolNames: () => [],
      setActiveToolsByName: () => {},
      navigateTree: async () => ({
        cancelled: false,
        aborted: false,
        editorText: "text",
      }),
      abortBranchSummary: () => {
        aborted = true;
      },
    };
    const runtime = {
      current: () => ({ session }) as never,
      requireIdle: () => ({ session }) as never,
      sessionGeneration: () => 1,
    } as unknown as RuntimeAccess;
    const events: JsonObject[] = [];
    const service = new TreeService(
      runtime,
      new PlanModeService(),
      new PlanStore(),
      new ContextBudgetManager(),
      (event) => events.push(event),
      () => ({ activation: true }),
      (snapshot) => ({ ...snapshot, scopeId: "session-1" }),
    );

    const state = service.state({
      filterMode: "default",
      query: "",
      foldedEntryIds: [],
    });
    assert.equal(state.leafId, null);

    manager.appendCustomMessageEntry("test.entry", "payload", false);
    const entryId = manager.getBranch()[0]!.id;
    service.label({ entryId, label: "renamed" });
    service.abort();
    assert.equal(aborted, true);

    const navigated = await service.navigate({
      entryId,
      summarize: false,
    });
    assert.equal(navigated.cancelled, false);
    assert.deepEqual(navigated.activation, { activation: true });
    assert.ok(
      events.some((event) => event.type === "plan_mode_state"),
      "plan mode state restored",
    );
    assert.ok(events.some((event) => event.type === "plan_state"));
    assert.ok(events.some((event) => event.type === "context_budget"));
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("stale generation completion is dropped", async () => {
  const root = mkdtempSync(join(tmpdir(), "nabla-tree-service-"));
  try {
    const manager = SessionManager.create(root, root);
    const session = {
      isIdle: true,
      sessionManager: manager,
      getActiveToolNames: () => [],
      setActiveToolsByName: () => {},
      navigateTree: async () => ({
        cancelled: false,
        aborted: false,
        editorText: "text",
      }),
      abortBranchSummary: () => {},
    };
    let generation = 1;
    const runtime = {
      current: () => ({ session }) as never,
      requireIdle: () => ({ session }) as never,
      sessionGeneration: () => generation,
    } as unknown as RuntimeAccess;
    const service = new TreeService(
      runtime,
      new PlanModeService(),
      new PlanStore(),
      new ContextBudgetManager(),
      () => {},
      () => ({ ok: true }),
      (snapshot) => snapshot,
    );
    const navigating = service.navigate({ entryId: "x", summarize: false });
    generation = 2;
    const result = await navigating;
    assert.equal(result.cancelled, true);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
