import assert from "node:assert/strict";
import test from "node:test";

import {
  estimateCategories,
  estimateTextTokens,
  estimateTopConsumers,
  firstString,
  formatTokens,
  normalizeArguments,
  safeEstimateCategories,
  safeEstimateMessages,
  safeSlice,
  safeSummary,
} from "./estimator.ts";
import type { AgentMessage } from "./model.ts";

test("token estimation is ceil(length / 4) with an empty-string floor", () => {
  assert.equal(estimateTextTokens(""), 0);
  assert.equal(estimateTextTokens("abcd"), 1);
  assert.equal(estimateTextTokens("abcde"), 2);
  assert.equal(estimateTextTokens("a".repeat(4096)), 1024);
});

test("category estimates keep the fixed order and count messages", () => {
  const messages = [
    {
      role: "user",
      content: [{ type: "text", text: "x".repeat(8) }],
      timestamp: 1,
    },
    {
      role: "assistant",
      content: [{ type: "text", text: "y".repeat(4) }],
      timestamp: 1,
    },
    {
      role: "toolResult",
      toolCallId: "t1",
      toolName: "grep",
      content: [{ type: "text", text: "z".repeat(8) }],
      isError: false,
      timestamp: 1,
    },
    { customType: "state" },
  ] as unknown as AgentMessage[];

  const categories = estimateCategories(messages);
  assert.deepEqual(
    categories.map((category) => category.category),
    ["user", "assistant", "toolResult", "other"],
  );
  assert.deepEqual(
    categories.map((category) => [category.messageCount, category.estimatedTokens]),
    [[1, 2], [1, 1], [1, 2], [1, 6]],
  );
});

test("safe estimators fall back to empty estimates on malformed messages", () => {
  const throwing = {
    get role() {
      throw new Error("boom");
    },
  } as unknown as AgentMessage;
  assert.deepEqual(safeEstimateCategories([throwing]), [
    { category: "user", messageCount: 0, estimatedTokens: 0 },
    { category: "assistant", messageCount: 0, estimatedTokens: 0 },
    { category: "toolResult", messageCount: 0, estimatedTokens: 0 },
    { category: "other", messageCount: 0, estimatedTokens: 0 },
  ]);
  assert.equal(safeEstimateMessages([throwing]), 0);
});

test("top consumers sort by tokens and pair tool results with their call", () => {
  const messages = [
    {
      role: "user",
      content: [{ type: "text", text: "x".repeat(100) }],
      timestamp: 1,
    },
    {
      role: "assistant",
      content: [
        { type: "toolCall", id: "t1", name: "read", arguments: { path: "/workspace/src/a.ts" } },
      ],
      timestamp: 1,
    },
    {
      role: "toolResult",
      toolCallId: "t1",
      toolName: "read",
      content: [{ type: "text", text: "z".repeat(200) }],
      isError: false,
      timestamp: 2,
    },
  ] as unknown as AgentMessage[];

  const consumers = estimateTopConsumers(messages);
  assert.equal(consumers.length, 3);
  assert.equal(consumers[0]?.category, "toolResult");
  assert.equal(consumers[0]?.toolCallId, "t1");
  assert.match(consumers[0]?.label ?? "", /· path \/workspace\/src\/a\.ts/u);
  assert.equal(consumers[1]?.category, "user");
  assert.equal(consumers[2]?.category, "assistant");
  assert.ok(consumers[0]!.estimatedTokens > consumers[1]!.estimatedTokens);
});

test("argument normalization, summaries, and formatting are deterministic", () => {
  assert.deepEqual(
    normalizeArguments({ b: 1, a: { d: 2, c: 3 } }),
    { a: { c: 3, d: 2 }, b: 1 },
  );
  assert.deepEqual(
    normalizeArguments({ path: "a\\b//c/" }, "path"),
    { path: "a/b/c" },
  );
  assert.equal(firstString({ path: "", file: "x" }, ["path", "file"]), "x");
  assert.equal(safeSummary("  hello\nworld  "), "hello world");
  assert.equal(safeSummary("a".repeat(200)), `${"a".repeat(157)}…`);
  assert.equal(safeSlice("a😀b", 1, 3), "😀");
  assert.equal(safeSlice("a😀b", 1, 2), "");
  assert.equal(formatTokens(999), "999");
  assert.equal(formatTokens(1500), "1.5k");
  assert.equal(formatTokens(12_345), "12k");
  assert.equal(formatTokens(2_000_000), "2.0m");
});
