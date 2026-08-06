import assert from "node:assert/strict";
import test from "node:test";

import {
  HostEventPublisher,
  type OutboundHostEvent,
} from "./host-event-publisher.ts";
import type { HostEvent } from "./contracts.ts";

test("events with an existing scopeId are never overwritten", () => {
  const received: OutboundHostEvent[] = [];
  const publisher = new HostEventPublisher((event) => received.push(event));
  publisher.setScopeIdProvider(() => "session-current");
  publisher.publish({
    type: "workspace_state",
    scopeId: "session-original",
    resources: { revision: 1 } as never,
    agents: { revision: 1 } as never,
  });
  assert.equal(received[0]?.scopeId, "session-original");
});

test("missing scopeId is filled from the current runtime provider", () => {
  const received: OutboundHostEvent[] = [];
  const publisher = new HostEventPublisher((event) => received.push(event));
  publisher.setScopeIdProvider(() => "session-1");
  publisher.publish({ type: "host_warning", message: "w" });
  assert.deepEqual(received[0], { type: "host_warning", message: "w", scopeId: "session-1" });
});

test("no scopeId is fabricated when the current runtime is unavailable", () => {
  const received: OutboundHostEvent[] = [];
  const publisher = new HostEventPublisher((event) => received.push(event));
  publisher.publish({ type: "host_warning", message: "w" });
  assert.deepEqual(received[0], { type: "host_warning", message: "w" });
});

test("explicit scopeId options win over the provider", () => {
  const received: OutboundHostEvent[] = [];
  const publisher = new HostEventPublisher((event) => received.push(event));
  publisher.setScopeIdProvider(() => "session-provider");
  publisher.publish(
    { type: "host_warning", message: "w" },
    { scopeId: "session-explicit", delivery: "coalescible" },
  );
  assert.deepEqual(received[0], {
    type: "host_warning",
    message: "w",
    scopeId: "session-explicit",
  });
});

test("sink failures propagate to the caller", () => {
  const publisher = new HostEventPublisher(() => {
    throw new Error("sink failed");
  });
  assert.throws(
    () => publisher.publish({ type: "host_warning", message: "w" }),
    /sink failed/u,
  );
});

test("publish accepts every documented host event kind", () => {
  const received: OutboundHostEvent[] = [];
  const publisher = new HostEventPublisher((event) => received.push(event));
  const events: HostEvent[] = [
    { type: "plan_state", artifact: null },
    { type: "turn_timing", phase: "started", turnId: "t", startedAt: "now" },
    { type: "context_budget", snapshot: { revision: 1 } as never },
    { type: "agents_state", snapshot: { revision: 1 } as never },
    { type: "response", command: "x", success: true },
    { type: "host_protocol_error", error: "e" },
  ];
  for (const event of events) publisher.publish(event);
  assert.equal(received.length, events.length);
});
