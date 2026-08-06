import assert from "node:assert/strict";
import test from "node:test";

import type { ApprovalRequest } from "../../approval.ts";
import type { PlanQuestion } from "../../questions.ts";
import { InteractionBroker } from "./interaction-broker.ts";

const question: PlanQuestion = {
  id: "q1",
  prompt: "Continue?",
  options: [
    { id: "yes", label: "Yes" },
    { id: "no", label: "No" },
  ],
};

function approvalRequest(): ApprovalRequest {
  return {
    requestId: "request-1",
    toolCallId: "tool-1",
    sessionId: "session-1",
    workspaceId: "workspace-1",
    summary: "Test",
    risk: "normal" as const,
    intentDigest: "digest",
    availableDecisions: ["allow_once", "deny"],
    toolName: "bash",
    input: { command: "echo hi" },
  };
}

test("approval request resolves on reply and rejects duplicates", async () => {
  const broker = new InteractionBroker();
  const pending = broker.requestApproval(approvalRequest(), undefined, () => {});
  broker.replyApproval("request-1", "allow_once");
  assert.equal(await pending, "allow_once");
  assert.throws(
    () => broker.replyApproval("request-1", "deny"),
    /Approval request is no longer active/u,
  );
});

test("question request resolves on reply and rejects duplicates", async () => {
  const broker = new InteractionBroker();
  const pending = broker.requestQuestions(
    [question],
    undefined,
    () => {},
    () => {},
  );
  broker.replyQuestion("question-1", [
    { questionId: "q1", value: "Yes", optionId: "yes" },
  ]);
  assert.deepEqual(await pending, [
    { questionId: "q1", value: "Yes", optionId: "yes" },
  ]);
  assert.throws(
    () => broker.replyQuestion("question-1", []),
    /Question request is no longer active/u,
  );
});

test("cancelAll denies approvals and cancels questions on disconnect", async () => {
  const broker = new InteractionBroker();
  const approval = broker.requestApproval(approvalRequest(), undefined, () => {});
  const questionRequest = broker.requestQuestions(
    [question],
    undefined,
    () => {},
    () => {},
  );
  broker.cancelAll("Host control client disconnected");
  assert.equal(await approval, "deny");
  await assert.rejects(questionRequest, /disconnected/u);
});

test("abort signal races resolve without double replies", async () => {
  const broker = new InteractionBroker();
  const controller = new AbortController();
  const approval = broker.requestApproval(
    approvalRequest(),
    controller.signal,
    () => {},
  );
  controller.abort();
  assert.equal(await approval, "deny");
  assert.throws(
    () => broker.replyApproval("request-1", "allow_once"),
    /Approval request is no longer active/u,
  );
});

test("stale request ids are rejected without consuming newer requests", async () => {
  const broker = new InteractionBroker();
  const first = broker.requestApproval(
    { ...approvalRequest(), requestId: "request-1" },
    undefined,
    () => {},
  );
  broker.replyApproval("request-1", "deny");
  await first;
  const second = broker.requestApproval(
    { ...approvalRequest(), requestId: "request-2" },
    undefined,
    () => {},
  );
  assert.throws(
    () => broker.replyApproval("request-1", "allow_once"),
    /Approval request is no longer active/u,
  );
  broker.replyApproval("request-2", "allow_once");
  assert.equal(await second, "allow_once");
});
