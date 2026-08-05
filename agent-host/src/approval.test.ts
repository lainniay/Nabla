import assert from "node:assert/strict";
import test from "node:test";

import {
  ApprovalQueue,
  type ApprovalRequest,
} from "./approval.ts";

function approvalRequest(
  request: Pick<ApprovalRequest, "toolCallId" | "toolName" | "input"> &
    Partial<ApprovalRequest>,
): ApprovalRequest {
  return {
    requestId: `request-${request.toolCallId}`,
    sessionId: "session-1",
    workspaceId: "workspace-1",
    summary: "Permission required",
    risk: "normal",
    intentDigest: `digest-${request.toolCallId}`,
    availableDecisions: ["allow_once", "deny"],
    ...request,
  };
}

test("approval queue waits for an explicit allow decision", async () => {
  const queue = new ApprovalQueue();
  let event: Record<string, unknown> | undefined;
  let settled = false;
  const result = queue.request(
    approvalRequest({
      toolCallId: "call-1",
      toolName: "bash",
      input: { command: "cargo test" },
    }),
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
  assert.equal(event?.requestId, "request-call-1");
  assert.equal(queue.reply(String(event?.requestId), "allow_once"), true);
  assert.equal(await result, "allow_once");
});

test("approval queue fails closed on abort, disconnect, and stale replies", async () => {
  const queue = new ApprovalQueue();
  const controller = new AbortController();
  const aborted = queue.request(
    approvalRequest({
      toolCallId: "call-1",
      toolName: "write",
      input: { path: "src/lib.rs" },
    }),
    controller.signal,
    () => {},
  );
  controller.abort();
  assert.equal(await aborted, "deny");

  const disconnected = queue.request(
    approvalRequest({
      toolCallId: "call-2",
      toolName: "edit",
      input: { path: "src/main.rs" },
    }),
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
    approvalRequest({
      toolCallId: "call-3",
      toolName: "bash",
      input: { command: "cargo test" },
      agentId: "agent-1",
      agentProfile: "verifier",
      model: "provider/model",
      goalId: "goal-1",
      reason: "Command is outside the current lease",
      risk: "normal",
      availableDecisions: ["allow_once", "allow_session", "deny"],
    }),
    undefined,
    (value) => {
      event = value;
    },
  );
  await Promise.resolve();
  assert.equal(event?.agentProfile, "verifier");
  assert.equal(event?.goalId, "goal-1");
  assert.equal(queue.reply(String(event?.requestId), "allow_session"), true);
  assert.equal(await result, "allow_session");
});

test("approval queue carries an explicit persistent decision", async () => {
  const queue = new ApprovalQueue();
  let event: Record<string, unknown> | undefined;
  const result = queue.request(
    approvalRequest({
      toolCallId: "tool-forever",
      toolName: "write",
      input: { path: "src/main.ts" },
      risk: "normal",
      availableDecisions: ["allow_once", "allow_workspace", "deny"],
    }),
    undefined,
    (value) => {
      event = value;
    },
  );
  assert.equal(queue.reply(String(event?.requestId), "allow_workspace"), true);
  assert.equal(await result, "allow_workspace");
});

test("approval queue carries a session-scoped decision", async () => {
  const queue = new ApprovalQueue();
  let event: Record<string, unknown> | undefined;
  const result = queue.request(
    approvalRequest({
      toolCallId: "tool-session",
      toolName: "bash",
      input: { command: "wc -l src/*.rs" },
      risk: "normal",
      availableDecisions: ["allow_once", "allow_session", "deny"],
    }),
    undefined,
    (value) => {
      event = value;
    },
  );
  assert.equal(queue.reply(String(event?.requestId), "allow_session"), true);
  assert.equal(await result, "allow_session");
});

test("approval request round-trips host-owned decisions and grant proposals", async () => {
  const queue = new ApprovalQueue();
  let event: Record<string, unknown> | undefined;
  const sessionGrant = {
    scope: "session" as const,
    workspaceId: "workspace-1",
    sessionId: "session-1",
    matchers: [{
      kind: "exec" as const,
      executable: "cargo",
      argv: ["test"],
      cwd: "/workspace",
      environment: {},
    }],
  };
  const result = queue.request(
    approvalRequest({
      toolCallId: "tool-round-trip",
      toolName: "bash",
      input: { command: "cargo test" },
      summary: "Run cargo test",
      availableDecisions: ["allow_once", "allow_session", "deny"],
      sessionGrant,
    }),
    undefined,
    (value) => {
      event = value;
    },
  );

  assert.equal(event?.requestId, "request-tool-round-trip");
  assert.equal(event?.sessionId, "session-1");
  assert.equal(event?.workspaceId, "workspace-1");
  assert.equal(event?.summary, "Run cargo test");
  assert.equal(event?.intentDigest, "digest-tool-round-trip");
  assert.deepEqual(event?.availableDecisions, [
    "allow_once",
    "allow_session",
    "deny",
  ]);
  assert.deepEqual(event?.sessionGrant, sessionGrant);
  assert.equal("approvalId" in (event ?? {}), false);
  assert.equal("grantProposals" in (event ?? {}), false);
  assert.equal(queue.reply(String(event?.requestId), "allow_session"), true);
  assert.equal(await result, "allow_session");
});

test("approval queue rejects an unknown decision without consuming the request", async () => {
  const queue = new ApprovalQueue();
  let event: Record<string, unknown> | undefined;
  const result = queue.request(
    approvalRequest({
      toolCallId: "tool-unknown",
      toolName: "bash",
      input: { command: "true" },
    }),
    undefined,
    (value) => {
      event = value;
    },
  );

  assert.throws(
    () => queue.reply(
      String(event?.requestId),
      "allow_forever" as never,
    ),
    /Unsupported approval decision/u,
  );
  assert.equal(queue.reply(String(event?.requestId), "deny"), true);
  assert.equal(await result, "deny");
});
