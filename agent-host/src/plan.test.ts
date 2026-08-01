import assert from "node:assert/strict";
import test from "node:test";

import {
  LEGACY_PLAN_ENTRY_TYPE,
  PLAN_MODE_ENTRY_TYPE,
  PlanStore,
  planExecutionPrompt,
  restorePlanMode,
} from "./plan.ts";

const content = {
  title: "Add structured plans",
  summary: "Treat plans as artifacts.",
  bodyMarkdown: "1. Add the host protocol.\n2. Render the review.",
  assumptions: ["Rust owns interaction"],
  testPlan: ["Run both suites"],
};

test("plan revisions keep stable identity and increment revision", () => {
  let tick = 0;
  const store = new PlanStore({
    createId: () => "plan-1",
    now: () => `2026-01-01T00:00:0${tick++}.000Z`,
  });

  const first = store.submit(content, "session-1");
  const second = store.submit({ ...content, summary: "Revised." }, "session-1");

  assert.equal(first.id, "plan-1");
  assert.equal(first.revision, 1);
  assert.equal(second.id, first.id);
  assert.equal(second.revision, 2);
  assert.equal(second.createdAt, first.createdAt);
  assert.equal(second.status, "submitted");
});

test("plan lifecycle timestamps stay monotonic when the clock does not advance", () => {
  const store = new PlanStore({
    createId: () => "plan-monotonic",
    now: () => "2026-01-01T00:00:00.000Z",
  });

  const submitted = store.submit(content, "session-1");
  const executing = store.markExecuting();
  const completed = store.markCompleted();

  assert.equal(submitted.updatedAt, "2026-01-01T00:00:00.000Z");
  assert.equal(executing.updatedAt, "2026-01-01T00:00:00.001Z");
  assert.equal(completed.updatedAt, "2026-01-01T00:00:00.002Z");
});

test("restore migrates legacy executing plans as interrupted and submitted", () => {
  const store = new PlanStore({ now: () => "2026-02-02T00:00:00.000Z" });
  const artifact = {
    schemaVersion: 1 as const,
    id: "plan-2",
    revision: 3,
    status: "executing" as const,
    ...content,
    sourceSessionId: "session-2",
    createdAt: "2026-01-01T00:00:00.000Z",
    updatedAt: "2026-01-01T00:00:01.000Z",
  };

  const restored = store.restore([
    { type: "message", data: { text: "not a plan" } },
    { type: "custom", customType: LEGACY_PLAN_ENTRY_TYPE, data: artifact },
  ]);

  assert.equal(restored.recovered, true);
  assert.equal(restored.artifact?.status, "submitted");
  assert.equal(restored.artifact?.revision, 3);
  assert.match(restored.artifact?.lastExecutionError ?? "", /interrupted/u);
});

test("execution prompt identifies the exact artifact revision", () => {
  const store = new PlanStore({
    createId: () => "plan-3",
    now: () => "2026-01-01T00:00:00.000Z",
  });
  const artifact = store.submit(content, "session-3");

  const prompt = planExecutionPrompt(artifact);

  assert.match(prompt, /PlanArtifact plan-3 revision 1/);
  assert.match(prompt, /Add the host protocol/);
  assert.match(prompt, /Run both suites/);
});

test("clearing the store starts an independent Plan identity", () => {
  const ids = ["plan-1", "plan-2"];
  const store = new PlanStore({
    createId: () => ids.shift() ?? "unexpected",
    now: () => "2026-01-01T00:00:00.000Z",
  });
  const first = store.submit(content, "session-1");
  store.clear();
  const second = store.submit(content, "session-1");
  assert.equal(first.id, "plan-1");
  assert.equal(second.id, "plan-2");
  assert.equal(second.revision, 1);
});

test("Plan mode restores from the active Pi branch only", () => {
  assert.equal(
    restorePlanMode([
      {
        type: "custom",
        customType: PLAN_MODE_ENTRY_TYPE,
        data: { active: true },
      },
      {
        type: "custom",
        customType: PLAN_MODE_ENTRY_TYPE,
        data: { active: false },
      },
    ]),
    false,
  );
});

test("Plan lifecycle rejects invalid status jumps and execution-time revision", () => {
  const store = new PlanStore({
    createId: () => "plan-state-machine",
    now: () => "2026-01-01T00:00:00.000Z",
  });
  store.submit(content, "session-1");
  assert.throws(() => store.markCompleted(), /cannot complete/u);

  store.markExecuting();
  assert.throws(() => store.markExecuting(), /cannot start/u);
  assert.throws(
    () => store.submit(content, "session-1"),
    /Cannot revise a Plan while it is executing/u,
  );
  store.markSubmitted("failed");
  assert.throws(() => store.markSubmitted(), /cannot return to submitted/u);

  store.markExecuting();
  store.markCompleted();
  assert.throws(() => store.markCompleted(), /cannot complete/u);
});
