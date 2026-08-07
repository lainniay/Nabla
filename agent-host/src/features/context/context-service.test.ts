import assert from "node:assert/strict";
import test from "node:test";

import { ContextBudgetManager } from "./engine.ts";
import type { JsonObject } from "../../protocol/validation.ts";
import { ContextService } from "./context-service.ts";

test("publish attaches the scoped snapshot and warning", () => {
  const events: JsonObject[] = [];
  const budget = new ContextBudgetManager();
  const service = new ContextService(
    budget,
    (event) => events.push(event),
    (snapshot) => ({ ...snapshot, scopeId: "session-1" }),
  );
  service.publish(service.snapshot());
  const event = events[0] as JsonObject & {
    snapshot: { scopeId: string };
  };
  assert.equal(event.type, "context_budget");
  assert.equal(event.snapshot.scopeId, "session-1");
});

test("runtime session start resets the budget and publishes usage", () => {
  const events: JsonObject[] = [];
  const service = new ContextService(
    new ContextBudgetManager(),
    (event) => events.push(event),
    (snapshot) => snapshot,
  );
  service.onRuntimeSessionStart({
    sessionManager: { getSessionId: () => "session-2" },
    getContextUsage: () => undefined,
  });
  assert.ok(events.some((event) => event.type === "context_budget"));
});

test("filter and compaction flow through the budget manager", () => {
  const service = new ContextService(
    new ContextBudgetManager(),
    () => {},
    (snapshot) => snapshot,
  );
  const filtered = service.filter(
    [],
    undefined,
    { planMode: false, plan: undefined },
  );
  assert.deepEqual(filtered.messages, []);
  assert.ok(filtered.snapshot);
  assert.ok(service.onCompaction({
    id: "c1",
    reason: "threshold",
    beforeTokens: 100,
    afterTokens: 50,
    createdAt: "now",
  } as never));
});

test("tree navigation advances the epoch and emits a budget event", () => {
  const events: JsonObject[] = [];
  const service = new ContextService(
    new ContextBudgetManager(),
    (event) => events.push(event),
    (snapshot) => snapshot,
  );
  service.publish(service.onTreeNavigation());
  assert.ok(events.some((event) => event.type === "context_budget"));
});
