import assert from "node:assert/strict";
import test from "node:test";

import { QuestionQueue, validateQuestions } from "./questions.ts";

const questions = [
  {
    id: "scope",
    prompt: "Which scope?",
    options: [
      { id: "minimal", label: "Minimal" },
      { id: "complete", label: "Complete" },
    ],
  },
];

test("question queue returns structured answers", async () => {
  const queue = new QuestionQueue();
  let requestId = "";
  const result = queue.request(
    questions,
    undefined,
    (id) => {
      requestId = id;
    },
    () => assert.fail("question should not be cancelled"),
  );

  assert.equal(
    queue.reply(requestId, [{ questionId: "scope", optionId: "complete", value: "Complete" }]),
    true,
  );
  assert.deepEqual(await result, [
    { questionId: "scope", optionId: "complete", value: "Complete" },
  ]);
});

test("question queue fails closed when the agent is aborted", async () => {
  const queue = new QuestionQueue();
  const controller = new AbortController();
  let cancelled = "";
  const result = queue.request(
    questions,
    controller.signal,
    () => {},
    (id) => {
      cancelled = id;
    },
  );

  controller.abort();

  await assert.rejects(result, /cancelled/);
  assert.match(cancelled, /^question-/);
});

test("question queue does not notify for an already aborted request", async () => {
  const queue = new QuestionQueue();
  const controller = new AbortController();
  controller.abort();
  let notified = false;
  let cancelled = false;

  const result = queue.request(
    questions,
    controller.signal,
    () => {
      notified = true;
    },
    () => {
      cancelled = true;
    },
  );

  await assert.rejects(result, /cancelled/);
  assert.equal(notified, false);
  assert.equal(cancelled, false);
});

test("question queue rejects unknown options without consuming the request", async () => {
  const queue = new QuestionQueue();
  let requestId = "";
  const result = queue.request(
    questions,
    undefined,
    (id) => {
      requestId = id;
    },
    () => {},
  );

  assert.throws(
    () =>
      queue.reply(requestId, [
        { questionId: "scope", optionId: "missing", value: "Missing" },
      ]),
    /Unknown option/,
  );
  assert.equal(
    queue.reply(requestId, [{ questionId: "scope", optionId: "minimal", value: "Minimal" }]),
    true,
  );
  await result;
});

test("question validation rejects duplicate question and option IDs", () => {
  assert.throws(
    () => validateQuestions([questions[0]!, questions[0]!]),
    /Duplicate question id/u,
  );
  assert.throws(
    () =>
      validateQuestions([
        {
          id: "duplicate-options",
          prompt: "Choose",
          options: [
            { id: "same", label: "First" },
            { id: "same", label: "Second" },
          ],
        },
      ]),
    /Duplicate option id/u,
  );
});

test("question replies reject duplicate answers without consuming the request", async () => {
  const queue = new QuestionQueue();
  const twoQuestions = [
    questions[0]!,
    {
      id: "mode",
      prompt: "Which mode?",
      options: [
        { id: "safe", label: "Safe" },
        { id: "fast", label: "Fast" },
      ],
    },
  ];
  let requestId = "";
  const result = queue.request(
    twoQuestions,
    undefined,
    (id) => {
      requestId = id;
    },
    () => {},
  );
  assert.throws(
    () =>
      queue.reply(requestId, [
        { questionId: "scope", optionId: "minimal", value: "Minimal" },
        { questionId: "scope", optionId: "complete", value: "Complete" },
      ]),
    /Duplicate answer/u,
  );
  assert.equal(
    queue.reply(requestId, [
      { questionId: "scope", optionId: "minimal", value: "Minimal" },
      { questionId: "mode", optionId: "safe", value: "Safe" },
    ]),
    true,
  );
  await result;
});
