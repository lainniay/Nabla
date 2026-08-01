import assert from "node:assert/strict";
import test from "node:test";

import {
  ContextBudgetManager,
  compactionRecordFromEntry,
  type ContextActiveState,
  type ContextPolicy,
} from "./context-manager.ts";
import type { PlanArtifactV2 } from "./plan.ts";

type Messages = Parameters<ContextBudgetManager["filter"]>[0];

const active: ContextActiveState = { planMode: false };

function policy(overrides: Partial<ContextPolicy> = {}): Partial<ContextPolicy> {
  return {
    recentToolResultTokens: 0,
    minimumBatchSavingsTokens: 0,
    ...overrides,
  };
}

function textResult(
  id: string,
  name: string,
  text: string,
  isError = false,
): Messages[number] {
  return {
    role: "toolResult",
    toolCallId: id,
    toolName: name,
    content: [{ type: "text", text }],
    details:
      name === "bash" ? { fullOutputPath: `/tmp/pi-${id}.log` } : undefined,
    isError,
    timestamp: 2,
  } as Messages[number];
}

function imageResult(id: string, text: string): Messages[number] {
  return {
    role: "toolResult",
    toolCallId: id,
    toolName: "read",
    content: [
      { type: "text", text },
      { type: "image", data: "base64", mimeType: "image/png" },
    ],
    isError: false,
    timestamp: 2,
  } as Messages[number];
}

function calls(
  ...items: Array<[id: string, name: string, args?: Record<string, unknown>]>
): Messages[number] {
  return {
    role: "assistant",
    content: items.map(([id, name, args = {}]) => ({
      type: "toolCall",
      id,
      name,
      arguments: args,
    })),
    timestamp: 1,
  } as Messages[number];
}

function resultText(message: Messages[number]): string {
  const content = (message as { content: Array<{ type: string; text?: string }> })
    .content;
  return content
    .filter((part) => part.type === "text")
    .map((part) => part.text ?? "")
    .join("\n");
}

function resultById(messages: Messages, id: string): Messages[number] {
  return messages.find(
    (message) =>
      (message as { role?: string }).role === "toolResult" &&
      (message as { toolCallId?: string }).toolCallId === id,
  )!;
}

function plan(): PlanArtifactV2 {
  return {
    schemaVersion: 2,
    id: "plan-1",
    revision: 3,
    status: "executing",
    title: "Keep state",
    summary: "Restore active state after compaction.",
    bodyMarkdown: "Implement the complete structured checkpoint.",
    assumptions: ["Pi remains authoritative"],
    testPlan: ["npm test", "cargo test"],
    sourceSessionId: "session-1",
    createdAt: "2026-01-01T00:00:00.000Z",
    updatedAt: "2026-01-01T00:00:01.000Z",
  };
}

test("hard limits preserve input, tool pairing, ratios, and Bash full-output hints", () => {
  const manager = new ContextBudgetManager({
    policy: policy({
      successToolResultLimitTokens: 100,
      searchToolResultLimitTokens: 80,
      errorToolResultLimitTokens: 90,
      minimumBatchSavingsTokens: 1_000_000,
    }),
  });
  const messages = [
    calls(
      ["ordinary", "bash", { command: "large output" }],
      ["search", "grep", { pattern: "needle", path: "src" }],
      ["failure", "read", { path: "missing" }],
    ),
    textResult("ordinary", "bash", "H".repeat(2_000)),
    textResult("search", "grep", "S".repeat(2_000)),
    textResult("failure", "read", "E".repeat(2_000), true),
  ] as Messages;
  const original = structuredClone(messages);

  const filtered = manager.filter(messages, undefined, active);

  assert.deepEqual(messages, original);
  assert.equal(filtered.messages.length, messages.length);
  for (const id of ["ordinary", "search", "failure"]) {
    assert.equal(
      (resultById(filtered.messages, id) as { toolCallId: string }).toolCallId,
      id,
    );
  }

  const success = resultText(resultById(filtered.messages, "ordinary"));
  const search = resultText(resultById(filtered.messages, "search"));
  const failure = resultText(resultById(filtered.messages, "failure"));
  assert.ok(success.length <= 400);
  assert.ok(search.length <= 320);
  assert.ok(failure.length <= 360);
  assert.match(success, /Pi full output: \/tmp\/pi-ordinary\.log/);

  const successHead = success.match(/^H+/u)?.[0].length ?? 0;
  const successTail = success.match(/H+$/u)?.[0].length ?? 0;
  assert.ok(successHead > successTail, "success should retain more head than tail");
  const failureHead = failure.match(/^E+/u)?.[0].length ?? 0;
  const failureTail = failure.match(/E+$/u)?.[0].length ?? 0;
  assert.ok(failureHead < failureTail, "failures should retain more tail than head");

  const stats = filtered.snapshot.pruning.find(
    (entry) => entry.reason === "hard_limit",
  );
  assert.equal(stats?.count, 3);
  assert.ok((stats?.estimatedTokensSaved ?? 0) > 1_000);
});

test("any image block protects the complete tool result", () => {
  const manager = new ContextBudgetManager({
    policy: policy({
      successToolResultLimitTokens: 10,
      minimumBatchSavingsTokens: 0,
    }),
  });
  const huge = "visual evidence ".repeat(1_000);
  const messages = [
    calls(["image", "read", { path: "diagram.png" }]),
    imageResult("image", huge),
  ] as Messages;

  const result = manager.filter(messages, undefined, active);

  assert.equal(resultText(result.messages[1]), huge);
  assert.deepEqual(result.messages[1], messages[1]);
});

test("question and plan artifacts are protected from history pruning", () => {
  const manager = new ContextBudgetManager({
    policy: policy({ successToolResultLimitTokens: 10_000 }),
  });
  const question = "structured answers ".repeat(100);
  const submittedPlan = "complete structured plan ".repeat(100);
  const messages = [
    calls(
      ["question", "ask_user", { questions: [] }],
      ["plan", "submit_plan", { title: "Plan" }],
    ),
    textResult("question", "ask_user", question),
    textResult("plan", "submit_plan", submittedPlan),
  ] as Messages;

  const result = manager.filter(messages, undefined, {
    planMode: true,
    plan: plan(),
  });

  assert.equal(resultText(resultById(result.messages, "question")), question);
  assert.equal(resultText(resultById(result.messages, "plan")), submittedPlan);
});

test("recent protection, small-result floor, batch threshold, and sticky epochs interact conservatively", () => {
  const manager = new ContextBudgetManager({
    policy: policy({
      recentToolResultTokens: 120,
      minimumBatchSavingsTokens: 100,
      minimumToolResultTokens: 50,
      successToolResultLimitTokens: 10_000,
    }),
  });
  const first = "A".repeat(400); // 100 tokens
  const second = "B".repeat(400);
  const newest = "C".repeat(480); // fills the protected 120-token suffix
  const small = "tiny".repeat(20); // 20 tokens
  const initial = [
    calls(
      ["first", "read", { path: "a" }],
      ["newest", "read", { path: "c" }],
      ["small", "read", { path: "small" }],
    ),
    textResult("first", "read", first),
    textResult("newest", "read", newest),
    textResult("small", "read", small),
  ] as Messages;

  const belowThreshold = manager.filter(initial, undefined, active);
  assert.equal(resultText(resultById(belowThreshold.messages, "first")), first);
  assert.ok(
    belowThreshold.snapshot.estimatedCurrentlyPrunableTokens > 0,
    "an unapplied candidate should remain visible in the snapshot",
  );
  assert.equal(resultText(resultById(belowThreshold.messages, "newest")), newest);
  assert.equal(resultText(resultById(belowThreshold.messages, "small")), small);

  const batch = [
    calls(
      ["first", "read", { path: "a" }],
      ["second", "read", { path: "b" }],
      ["newest", "read", { path: "c" }],
    ),
    textResult("first", "read", first),
    textResult("second", "read", second),
    textResult("newest", "read", newest),
  ] as Messages;
  const applied = manager.filter(batch, undefined, active);
  assert.match(resultText(resultById(applied.messages, "first")), /Nabla pruned/);
  assert.match(resultText(resultById(applied.messages, "second")), /Nabla pruned/);
  assert.equal(resultText(resultById(applied.messages, "newest")), newest);

  const stickyNowRecent = [
    calls(["first", "read", { path: "a" }]),
    textResult("first", "read", first),
  ] as Messages;
  const sticky = manager.filter(stickyNowRecent, undefined, active);
  assert.match(resultText(sticky.messages[1]), /Nabla pruned/);

  manager.onCompaction(
    compactionRecordFromEntry("manual", {
      firstKeptEntryId: "kept",
      tokensBefore: 1_000,
    }),
  );
  const reset = manager.filter(stickyNowRecent, undefined, active);
  assert.equal(resultText(reset.messages[1]), first);
  assert.equal(reset.snapshot.usageState, "recalculating");
});

test("switching sessions starts a clean epoch and clears cumulative request accounting", () => {
  const manager = new ContextBudgetManager({
    policy: policy({
      recentToolResultTokens: 120,
      minimumBatchSavingsTokens: 100,
      successToolResultLimitTokens: 10_000,
    }),
  });
  manager.onSessionStart("session-1");
  const content = "evidence".repeat(50);
  const batch = [
    calls(
      ["first", "read", { path: "first" }],
      ["second", "read", { path: "second" }],
      ["recent", "read", { path: "recent" }],
    ),
    textResult("first", "read", content),
    textResult("second", "read", content),
    textResult("recent", "read", "R".repeat(480)),
  ] as Messages;
  const applied = manager.filter(batch, undefined, active);
  assert.ok(applied.snapshot.estimatedCumulativeAvoidedTokens > 0);

  const switched = manager.onSessionStart("session-2");
  assert.equal(switched.estimatedCumulativeAvoidedTokens, 0);
  assert.equal(switched.compactionCount, 0);
  assert.equal(switched.epoch, 2);

  const nowRecent = [
    calls(["first", "read", { path: "first" }]),
    textResult("first", "read", content),
  ] as Messages;
  assert.equal(
    resultText(manager.filter(nowRecent, undefined, active).messages[1]),
    content,
  );
});

test("tree navigation resets sticky pruning without pretending a compaction occurred", () => {
  const manager = new ContextBudgetManager({
    policy: policy({
      recentToolResultTokens: 120,
      minimumBatchSavingsTokens: 100,
      successToolResultLimitTokens: 10_000,
    }),
  });
  const content = "branch evidence ".repeat(40);
  const batch = [
    calls(
      ["first", "read", { path: "first" }],
      ["second", "read", { path: "second" }],
      ["recent", "read", { path: "recent" }],
    ),
    textResult("first", "read", content),
    textResult("second", "read", content),
    textResult("recent", "read", "R".repeat(480)),
  ] as Messages;
  const applied = manager.filter(batch, undefined, active);
  assert.match(resultText(resultById(applied.messages, "first")), /Nabla pruned/);
  const cumulative = applied.snapshot.estimatedCumulativeAvoidedTokens;
  const originalEpoch = applied.snapshot.epoch;

  const navigated = manager.onTreeNavigation();
  assert.equal(navigated.epoch, originalEpoch + 1);
  assert.equal(navigated.usageState, "recalculating");
  assert.equal(navigated.actualTokens, null);
  assert.equal(navigated.estimatedSystemToolOtherTokens, null);
  assert.equal(navigated.estimatedCumulativeAvoidedTokens, cumulative);
  assert.equal(navigated.compactionCount, 0);
  assert.deepEqual(navigated.recentCompactions, []);

  const nowRecent = [
    calls(["first", "read", { path: "first" }]),
    textResult("first", "read", content),
  ] as Messages;
  assert.equal(
    resultText(manager.filter(nowRecent, undefined, active).messages[1]),
    content,
  );
});

test("supersession requires exact normalized arguments and no intervening mutation", () => {
  const content = "positive evidence ".repeat(100);
  const manager = new ContextBudgetManager({
    policy: policy({
      recentToolResultTokens: 400,
      successToolResultLimitTokens: 10_000,
    }),
  });
  const messages = [
    calls(["old", "read", { path: "src/" }]),
    textResult("old", "read", content),
    calls(["new", "read", { path: "src" }]),
    textResult("new", "read", content),
  ] as Messages;

  const result = manager.filter(messages, undefined, active);
  assert.match(
    resultText(resultById(result.messages, "old")),
    /superseded by a later identical successful call/,
  );
  assert.equal(resultText(resultById(result.messages, "new")), content);
  assert.equal(
    result.snapshot.pruning.find((entry) => entry.reason === "superseded")
      ?.count,
    1,
  );

  const barrierManager = new ContextBudgetManager({
    policy: policy({
      recentToolResultTokens: 400,
      successToolResultLimitTokens: 10_000,
    }),
  });
  const withBarrier = [
    calls(["old", "read", { path: "src" }]),
    textResult("old", "read", content),
    calls(["write", "write", { path: "src/lib.ts", content: "changed" }]),
    textResult("write", "write", "done"),
    calls(["new", "read", { path: "src" }]),
    textResult("new", "read", content),
  ] as Messages;
  const barrier = barrierManager.filter(withBarrier, undefined, active);
  assert.doesNotMatch(
    resultText(resultById(barrier.messages, "old")),
    /superseded/,
  );
});

test("negative searches and old errors are never classified as superseded", () => {
  const manager = new ContextBudgetManager({
    policy: policy({
      recentToolResultTokens: 100,
      successToolResultLimitTokens: 10_000,
    }),
  });
  const messages = [
    calls(["empty", "grep", { pattern: "missing", path: "src" }]),
    textResult("empty", "grep", "No matches found"),
    calls(["error", "grep", { pattern: "missing", path: "src" }]),
    textResult("error", "grep", "permission denied", true),
    calls(["later", "grep", { pattern: "missing", path: "src" }]),
    textResult("later", "grep", "src/lib.ts:1:missing"),
  ] as Messages;

  const result = manager.filter(messages, undefined, active);

  assert.doesNotMatch(
    resultText(resultById(result.messages, "empty")),
    /superseded/,
  );
  assert.doesNotMatch(
    resultText(resultById(result.messages, "error")),
    /superseded/,
  );
});

test("checkpoint is stable, model-only, structured, and avoids a duplicate plan revision", () => {
  let timestamp = 100;
  const manager = new ContextBudgetManager({
    policy: policy(),
    now: () => timestamp++,
  });
  const artifact = plan();
  const base = [
    {
      role: "compactionSummary",
      summary: "Earlier work.",
      tokensBefore: 100_000,
      timestamp: 1,
    },
    {
      role: "user",
      content: "continue",
      timestamp: 2,
    },
  ] as Messages;

  const first = manager.filter(base, undefined, {
    planMode: false,
    plan: artifact,
  });
  const checkpoint = first.messages[1] as unknown as {
    role: string;
    customType: string;
    content: string;
    display: boolean;
    timestamp: number;
  };
  assert.equal(checkpoint.role, "custom");
  assert.equal(checkpoint.customType, "nabla.context-checkpoint");
  assert.equal(checkpoint.display, false);
  assert.match(checkpoint.content, /"planMode":false/);
  assert.match(checkpoint.content, /"bodyMarkdown"/);
  assert.equal(first.messages[2], first.messages[2]);

  const again = manager.filter(base, undefined, {
    planMode: false,
    plan: artifact,
  });
  assert.deepEqual(again.messages[1], first.messages[1]);

  const alreadyPresent = [
    base[0],
    {
      role: "custom",
      customType: "nabla.plan.execution.v1",
      content: "the full plan is already here",
      display: false,
      details: { planId: artifact.id, revision: artifact.revision },
      timestamp: 3,
    },
  ] as Messages;
  const deduplicated = manager.filter(alreadyPresent, undefined, {
    planMode: false,
    plan: artifact,
  });
  const deduplicatedText = (
    deduplicated.messages[1] as unknown as { content: string }
  ).content;
  assert.match(deduplicatedText, /planAlreadyPresent/);
  assert.doesNotMatch(deduplicatedText, /bodyMarkdown/);
});

test("checkpoint carries only the stable host goal view after compaction", () => {
  const manager = new ContextBudgetManager({ policy: policy(), now: () => 42 });
  const messages = [
    {
      role: "compactionSummary",
      summary: "Earlier work.",
      tokensBefore: 100_000,
      timestamp: 1,
    },
  ] as Messages;
  const goal = {
    id: "goal-1",
    revision: 7,
    objective: "Implement the harness",
    stage: "executing",
    currentTasks: [{ id: "host", status: "running" }],
  };

  const result = manager.filter(messages, undefined, {
    planMode: false,
    goal,
  });
  const checkpoint = result.messages[1] as unknown as {
    content: string;
    display: boolean;
    details: Record<string, unknown>;
  };
  assert.equal(checkpoint.display, false);
  assert.match(checkpoint.content, /"goal-1"/u);
  assert.match(checkpoint.content, /"currentTasks"/u);
  assert.doesNotMatch(checkpoint.content, /statePath|reviews|fullOutput/u);
  assert.equal(checkpoint.details.epoch, 0);
  assert.equal(checkpoint.details.goalId, "goal-1");
  assert.equal(checkpoint.details.goalRevision, 7);
});

test("unknown messages fail open once and disabled pruning returns the original view", () => {
  const manager = new ContextBudgetManager({ policy: policy() });
  const unknown = [{ role: "futurePiMessage", payload: "new" }] as unknown as Messages;

  const first = manager.filter(unknown, undefined, active);
  assert.equal(first.messages, unknown);
  assert.match(manager.takeWarning() ?? "", /not recognized/);

  manager.filter(unknown, undefined, active);
  assert.equal(manager.takeWarning(), undefined);

  const disabled = new ContextBudgetManager({
    policy: { ...policy(), enabled: false },
  });
  const compacted = [
    {
      role: "compactionSummary",
      summary: "summary",
      tokensBefore: 1_000,
      timestamp: 1,
    },
  ] as Messages;
  const unchanged = disabled.filter(compacted, undefined, active);
  assert.equal(unchanged.messages, compacted);
  assert.equal(unchanged.messages.length, 1);
});

test("environment policy parsing accepts documented booleans and warns once for invalid numbers", () => {
  const off = new ContextBudgetManager({
    env: {
      NABLA_CONTEXT_PRUNING: "0",
      NABLA_CONTEXT_PROTECTED_TOKENS: "-1",
      NABLA_CONTEXT_MIN_PRUNE_TOKENS: "twenty",
    },
  });
  assert.equal(off.snapshot().policy.enabled, false);
  assert.equal(off.snapshot().policy.recentToolResultTokens, 40_000);
  assert.equal(off.snapshot().policy.minimumBatchSavingsTokens, 20_000);
  const warning = off.takeWarning() ?? "";
  assert.match(warning, /PROTECTED_TOKENS/);
  assert.match(warning, /MIN_PRUNE_TOKENS/);
  assert.equal(off.takeWarning(), undefined);

  for (const value of ["on", "true", "1", "ON"]) {
    const enabled = new ContextBudgetManager({
      env: { NABLA_CONTEXT_PRUNING: value },
    });
    assert.equal(enabled.snapshot().policy.enabled, true);
  }
  for (const value of ["off", "false", "0", "OFF"]) {
    const disabled = new ContextBudgetManager({
      env: { NABLA_CONTEXT_PRUNING: value },
    });
    assert.equal(disabled.snapshot().policy.enabled, false);
  }
});

test("synthetic 100k-token output shrinks only the model view and accumulates per request", () => {
  const manager = new ContextBudgetManager({
    policy: policy({
      successToolResultLimitTokens: 12_000,
      minimumBatchSavingsTokens: 20_000,
    }),
  });
  const originalOutput = "x".repeat(400_000);
  const messages = [
    calls(["huge", "read", { path: "generated.log" }]),
    textResult("huge", "read", originalOutput),
  ] as Messages;
  const sessionEntry = structuredClone(messages[1]);

  const first = manager.filter(messages, undefined, active);
  assert.equal(resultText(messages[1]), originalOutput);
  assert.deepEqual(messages[1], sessionEntry);
  assert.ok(first.snapshot.estimatedNextRequestTokens < 13_000);
  const firstCumulative = first.snapshot.estimatedCumulativeAvoidedTokens;
  assert.ok(firstCumulative > 87_000);

  const second = manager.filter(messages, undefined, active);
  assert.ok(
    second.snapshot.estimatedCumulativeAvoidedTokens > firstCumulative * 1.9,
  );
});

test("actual usage becomes recalculating after compact and realigns on the next response", () => {
  const manager = new ContextBudgetManager({ policy: policy() });
  const messages = [
    { role: "user", content: "hello", timestamp: 1 },
  ] as Messages;
  manager.filter(
    messages,
    { tokens: 47_000, contextWindow: 100_000, percent: 47 },
    active,
  );
  manager.onModelResponse({
    tokens: 47_000,
    contextWindow: 100_000,
    percent: 47,
  });
  assert.equal(manager.snapshot().usageState, "actual");

  manager.onCompaction(
    compactionRecordFromEntry(
      "threshold",
      {
        firstKeptEntryId: "entry-1",
        tokensBefore: 82_000,
        details: {
          readFiles: ["a", "b"],
          modifiedFiles: ["b", "c"],
        },
      },
      31_000,
    ),
  );
  const compacted = manager.snapshot();
  assert.equal(compacted.usageState, "recalculating");
  assert.equal(compacted.actualTokens, null);
  assert.equal(compacted.compactionCount, 1);
  assert.equal(compacted.recentCompactions[0].readFileCount, 2);
  assert.equal(compacted.recentCompactions[0].fileCount, 3);
  assert.equal(compacted.recentCompactions[0].savedPercent, 62.19512195121951);

  manager.filter(messages, { tokens: null, contextWindow: 100_000, percent: null }, active);
  manager.onModelResponse({
    tokens: 31_500,
    contextWindow: 100_000,
    percent: 31.5,
  });
  assert.equal(manager.snapshot().usageState, "actual");
  assert.equal(manager.snapshot().actualPercent, 31.5);
  assert.notEqual(manager.snapshot().estimatedSystemToolOtherTokens, null);
});
