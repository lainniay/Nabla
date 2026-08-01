import assert from "node:assert/strict";
import test from "node:test";

import { AuthPromptQueue } from "./auth-prompts.ts";

test("auth prompt queue does not announce an already aborted prompt", async () => {
  const queue = new AuthPromptQueue();
  const controller = new AbortController();
  controller.abort();
  let announced = false;
  let cancelled = false;

  const result = queue.request(
    "prompt-1",
    [controller.signal],
    () => {
      announced = true;
    },
    () => {
      cancelled = true;
    },
  );

  await assert.rejects(result, /Login cancelled/);
  assert.equal(announced, false);
  assert.equal(cancelled, false);
  assert.equal(queue.reply("prompt-1", "ignored"), false);
});

test("auth prompt queue cancels an announced prompt exactly once", async () => {
  const queue = new AuthPromptQueue();
  const promptController = new AbortController();
  const flowController = new AbortController();
  let cancelled = 0;

  const result = queue.request(
    "prompt-1",
    [promptController.signal, flowController.signal],
    () => {},
    () => {
      cancelled += 1;
    },
  );

  flowController.abort();
  promptController.abort();

  await assert.rejects(result, /Login cancelled/);
  assert.equal(cancelled, 1);
  assert.equal(queue.reply("prompt-1", "ignored"), false);
});

test("auth prompt queue resolves replies and removes abort listeners", async () => {
  const queue = new AuthPromptQueue();
  const controller = new AbortController();
  let cancelled = false;
  const result = queue.request(
    "prompt-1",
    [controller.signal],
    () => {},
    () => {
      cancelled = true;
    },
  );

  assert.equal(queue.reply("prompt-1", "secret"), true);
  controller.abort();

  assert.equal(await result, "secret");
  assert.equal(cancelled, false);
});
