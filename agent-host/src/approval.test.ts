import assert from "node:assert/strict";
import test from "node:test";

import { ApprovalQueue } from "./approval.ts";

test("approval queue waits for an explicit allow decision", async () => {
  const queue = new ApprovalQueue();
  let event: Record<string, unknown> | undefined;
  let settled = false;
  const result = queue.request(
    { toolCallId: "call-1", toolName: "bash", input: { command: "cargo test" } },
    undefined,
    (value) => {
      event = value;
    },
  );
  void result.then(() => {
    settled = true;
  });

  await Promise.resolve();
  assert.equal(settled, false);
  assert.equal(event?.type, "approval_request");
  assert.equal(queue.reply(String(event?.approvalId), "allow_once"), true);
  assert.equal(await result, "allow_once");
});

test("approval queue fails closed on abort, disconnect, and stale replies", async () => {
  const queue = new ApprovalQueue();
  const controller = new AbortController();
  const aborted = queue.request(
    { toolCallId: "call-1", toolName: "write", input: { path: "src/lib.rs" } },
    controller.signal,
    () => {},
  );
  controller.abort();
  assert.equal(await aborted, "deny");

  const disconnected = queue.request(
    { toolCallId: "call-2", toolName: "edit", input: { path: "src/main.rs" } },
    undefined,
    () => {},
  );
  queue.denyAll();
  assert.equal(await disconnected, "deny");
  assert.equal(queue.reply("approval-missing", "allow_once"), false);
});

test("approval queue preserves agent and Goal metadata for a scoped decision", async () => {
  const queue = new ApprovalQueue();
  let event: Record<string, unknown> | undefined;
  const result = queue.request(
    {
      toolCallId: "call-3",
      toolName: "bash",
      input: { command: "cargo test" },
      agentId: "agent-1",
      agentProfile: "verifier",
      model: "provider/model",
      goalId: "goal-1",
      reason: "Command is outside the current lease",
      risk: "normal",
    },
    undefined,
    (value) => {
      event = value;
    },
  );
  await Promise.resolve();
  assert.equal(event?.agentProfile, "verifier");
  assert.equal(event?.goalId, "goal-1");
  assert.equal(queue.reply(String(event?.approvalId), "allow_session"), true);
  assert.equal(await result, "allow_session");
});

test("approval queue carries an explicit persistent decision", async () => {
  const queue = new ApprovalQueue();
  let event: Record<string, unknown> | undefined;
  const result = queue.request(
    {
      toolCallId: "tool-forever",
      toolName: "write",
      input: { path: "src/main.ts" },
      risk: "normal",
    },
    undefined,
    (value) => {
      event = value;
    },
  );
  assert.equal(queue.reply(String(event?.approvalId), "allow_workspace"), true);
  assert.equal(await result, "allow_workspace");
});

test("approval queue carries a session-scoped decision", async () => {
  const queue = new ApprovalQueue();
  let event: Record<string, unknown> | undefined;
  const result = queue.request(
    {
      toolCallId: "tool-session",
      toolName: "bash",
      input: { command: "wc -l src/*.rs" },
      risk: "normal",
    },
    undefined,
    (value) => {
      event = value;
    },
  );
  assert.equal(queue.reply(String(event?.approvalId), "allow_session"), true);
  assert.equal(await result, "allow_session");
});
