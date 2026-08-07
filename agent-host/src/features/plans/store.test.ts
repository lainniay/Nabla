import assert from "node:assert/strict";
import test from "node:test";

import {
  PLAN_ENTRY_TYPE,
  PLAN_MODE_ENTRY_TYPE,
  type PlanArtifact,
  planImplementationPrompt,
  planRevisionMarker,
} from "./model.ts";
import { PlanStore, restorePlanMode } from "./store.ts";

const content = {
  title: "Add structured plans",
  summary: "Treat plans as artifacts.",
  bodyMarkdown: "1. Add the host protocol.\n2. Render the review.",
  assumptions: ["Rust owns interaction"],
  testPlan: ["Run both suites"],
  handoffMarkdown: "Keep the artifact immutable across sessions.",
};

function artifact(overrides: Partial<PlanArtifact> = {}): PlanArtifact {
  return {
    id: "plan-1",
    revision: 1,
    ...content,
    sourceSessionId: "session-1",
    createdAt: "2026-01-01T00:00:00.000Z",
    updatedAt: "2026-01-01T00:00:00.000Z",
    ...overrides,
  };
}

test("plan revisions keep stable identity and increment revision without status", () => {
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
  assert.ok(second.updatedAt > first.updatedAt);
  assert.equal("status" in second, false);
});

test("plan artifact timestamps stay monotonic when the clock does not advance", () => {
  const store = new PlanStore({
    createId: () => "plan-monotonic",
    now: () => "2026-01-01T00:00:00.000Z",
  });

  const first = store.submit(content, "session-1");
  const second = store.submit(content, "session-1");

  assert.equal(first.updatedAt, "2026-01-01T00:00:00.000Z");
  assert.equal(second.updatedAt, "2026-01-01T00:00:00.001Z");
});

test("restore reads only the last valid nabla.plan entry and ignores legacy entries", () => {
  const store = new PlanStore();
  const legacyTypes = [1, 2].map((version) => `nabla.plan.v${version}`);
  const restored = store.restore([
    { type: "message", data: { text: "not a plan" } },
    { type: "custom", customType: legacyTypes[0], data: artifact() },
    { type: "custom", customType: PLAN_ENTRY_TYPE, data: artifact() },
    { type: "custom", customType: PLAN_ENTRY_TYPE, data: artifact({ revision: 2 }) },
    { type: "custom", customType: PLAN_ENTRY_TYPE, data: { id: "broken" } },
  ]);

  assert.equal(restored?.id, "plan-1");
  assert.equal(restored?.revision, 2);
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

test("handoffMarkdown must be non-empty when submitting", () => {
  const store = new PlanStore();
  assert.throws(
    () => store.submit({ ...content, handoffMarkdown: "  " }, "session-1"),
    /handoffMarkdown/u,
  );
});

test("plan content and lists are trimmed and normalized on submit", () => {
  const store = new PlanStore();
  const submitted = store.submit(
    {
      title: "  Padded title  ",
      summary: "  Summary  ",
      bodyMarkdown: "  Body  ",
      assumptions: ["  first  ", "", "second"],
      testPlan: ["  cargo test  "],
      handoffMarkdown: "  Handoff  ",
    },
    "session-1",
  );

  assert.deepEqual(submitted, {
    id: submitted.id,
    revision: 1,
    title: "Padded title",
    summary: "Summary",
    bodyMarkdown: "Body",
    assumptions: ["first", "second"],
    testPlan: ["cargo test"],
    handoffMarkdown: "Handoff",
    sourceSessionId: "session-1",
    createdAt: submitted.createdAt,
    updatedAt: submitted.updatedAt,
  });
});

test("implementation prompt carries the full artifact and handoff without lifecycle fields", () => {
  const prompt = planImplementationPrompt(artifact({ revision: 3 }));

  assert.match(prompt, /Plan plan-1 revision 3/);
  assert.match(prompt, /nabla-plan-artifact:plan-1:3/);
  assert.match(prompt, /## Source objective and handoff/);
  assert.match(prompt, /Keep the artifact immutable across sessions\./);
  assert.match(prompt, /## Approved plan/);
  assert.match(prompt, /Add the host protocol/);
  assert.match(prompt, /Run both suites/);
  assert.doesNotMatch(prompt, /status|completed|executing/iu);
});

test("plan revision marker changes with the revision", () => {
  assert.notEqual(
    planRevisionMarker("plan-1", 3),
    planRevisionMarker("plan-1", 4),
  );
  assert.equal(planRevisionMarker("plan-1", 3), "nabla-plan-artifact:plan-1:3");
});
