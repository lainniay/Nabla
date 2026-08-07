import assert from "node:assert/strict";
import test from "node:test";

import type {
  AgentSessionRuntime,
  ModelRuntime,
} from "@earendil-works/pi-coding-agent";

import {
  PLAN_ENTRY_TYPE,
  PLAN_MODE_ENTRY_TYPE,
  type PlanArtifact,
  type PlanContent,
} from "./model.ts";
import { PlanStore } from "./store.ts";
import {
  PLAN_TRANSFER_MAX_ABSOLUTE_TOKENS,
  executePlan,
  freshImplementationPrompt,
  transferBudget,
  type PlanExecutionDeps,
} from "./execution.ts";

const content: PlanContent = {
  title: "Structured planning",
  summary: "Treat plans as immutable artifacts.",
  bodyMarkdown: "Implement the artifact flow.",
  assumptions: ["Rust owns interaction"],
  testPlan: ["Run cargo test"],
  handoffMarkdown: "Preserve the artifact across sessions.",
};

function submittedArtifact(overrides: Partial<PlanArtifact> = {}): PlanArtifact {
  const store = new PlanStore();
  return store.submit({ ...content, ...overrides }, "session-1");
}

interface Entry {
  type: string;
  data?: unknown;
}

class StubSession {
  isIdle = true;
  model: Record<string, unknown> | undefined = {
    provider: "test",
    id: "model-1",
    contextWindow: 64_000,
  };
  thinkingLevel = "high";
  sessionFile: string | undefined = "/sessions/session-1.jsonl";
  sessionId = "session-1";
  entries: Entry[] = [];
  promptCalls: string[] = [];
  modelCalls: Array<Record<string, unknown> | undefined> = [];
  thinkingCalls: string[] = [];
  promptResult: Promise<void> = Promise.resolve();

  sessionManager = {
    appendCustomEntry: (type: string, data?: unknown) => {
      this.entries.push({ type, data });
    },
  };

  getActiveToolNames(): string[] {
    return ["read"];
  }

  async prompt(text: string): Promise<void> {
    this.promptCalls.push(text);
    return this.promptResult;
  }

  async setModel(model: Record<string, unknown> | undefined): Promise<void> {
    this.modelCalls.push(model);
  }

  setThinkingLevel(level: string): void {
    this.thinkingCalls.push(level);
  }
}

interface StubRuntime {
  session: StubSession;
  newSessionCalls: number;
  newSessionOptions?: {
    parentSession?: string;
    setup?: (sessionManager: StubSession["sessionManager"]) => Promise<void>;
  };
  newSessionCancelled: boolean;
  newSession(): Promise<{ cancelled: boolean }>;
}

function runtime(
  session: StubSession,
  overrides: Partial<StubRuntime> = {},
): StubRuntime {
  return {
    session,
    newSessionCalls: 0,
    newSessionCancelled: false,
    async newSession(this: StubRuntime, options?: {
      parentSession?: string;
      setup?: (sessionManager: StubSession["sessionManager"]) => Promise<void>;
    }) {
      this.newSessionCalls += 1;
      this.newSessionOptions = options;
      if (this.newSessionCancelled) return { cancelled: true };
      const fresh = new StubSession();
      fresh.sessionId = "session-2";
      fresh.sessionFile = "/sessions/session-2.jsonl";
      fresh.model = session.model;
      fresh.thinkingLevel = session.thinkingLevel;
      if (options?.setup) {
        await options.setup(fresh.sessionManager);
      }
      this.session = fresh;
      return { cancelled: false };
    },
    ...overrides,
  };
}

function deps(overrides: {
  artifact?: PlanArtifact;
  runtime?: StubRuntime;
  availableModel?: Record<string, unknown> | undefined;
} = {}) {
  const plans = new PlanStore();
  plans.submit(content, "session-1");
  const session = new StubSession();
  const stubRuntime = runtime(session);
  const sent: Array<Record<string, unknown>> = [];
  const modeCalls: boolean[] = [];
  const errors: unknown[] = [];
  const dependencies: PlanExecutionDeps = {
    plans,
    modelRuntime: {
      getModel: (provider: string, id: string) =>
        overrides.availableModel === undefined && provider === "test" && id === "model-1"
          ? session.model
          : overrides.availableModel,
    } as unknown as ModelRuntime,
    runtime: () => stubRuntime as unknown as AgentSessionRuntime,
    setPlanMode: (active) => {
      modeCalls.push(active);
      session.sessionManager.appendCustomEntry(PLAN_MODE_ENTRY_TYPE, {
        active,
      });
      sent.push({ type: "plan_mode_state", active });
    },
    send: (message) => {
      sent.push(message as Record<string, unknown>);
    },
    reportTurnError: (error) => {
      errors.push(error);
    },
  };
  return {
    plans,
    session,
    stubRuntime,
    sent,
    modeCalls,
    errors,
    dependencies,
  };
}

test("transfer budget caps by context fraction and absolute ceiling", () => {
  assert.equal(transferBudget(32_000), 8_000);
  assert.equal(transferBudget(64_000), 16_000);
  assert.equal(transferBudget(128_000), 24_000);
  assert.equal(transferBudget(null), PLAN_TRANSFER_MAX_ABSOLUTE_TOKENS);
  assert.equal(transferBudget(undefined), PLAN_TRANSFER_MAX_ABSOLUTE_TOKENS);
});

test("fresh prompt includes the handoff and full Plan without the transcript", () => {
  const prompt = freshImplementationPrompt(submittedArtifact());
  assert.match(prompt, /fresh session/u);
  assert.match(prompt, /Preserve the artifact across sessions\./u);
  assert.match(prompt, /## Approved plan/u);
  assert.match(prompt, /Implement the artifact flow\./u);
  assert.doesNotMatch(prompt, /old planning transcript/u);
});

test("current execution starts a normal turn without waiting for it", async () => {
  const ctx = deps();
  let release!: () => void;
  ctx.session.promptResult = new Promise<void>((resolve) => {
    release = resolve;
  });

  const result = await executePlan("current", ctx.dependencies);

  assert.deepEqual(result, { sessionId: "session-1", context: "current" });
  assert.equal(ctx.session.promptCalls.length, 1);
  assert.match(ctx.session.promptCalls[0], /## Source objective and handoff/u);
  release();
});

test("current execution exits Plan mode and mutates nothing else", async () => {
  const ctx = deps();
  const before = ctx.plans.latest();

  await executePlan("current", ctx.dependencies);

  assert.deepEqual(ctx.modeCalls, [false]);
  assert.deepEqual(ctx.session.entries, [
    { type: PLAN_MODE_ENTRY_TYPE, data: { active: false } },
  ]);
  assert.ok(
    ctx.sent.some(
      (message) =>
        message.type === "plan_mode_state" && message.active === false,
    ),
  );
  assert.ok(ctx.sent.every((message) => message.type === "plan_mode_state"));
  assert.deepEqual(ctx.plans.latest(), before);
});

test("current execution fails before any mutation when busy or planless", async () => {
  const busy = deps();
  busy.session.isIdle = false;
  await assert.rejects(
    executePlan("current", busy.dependencies),
    /agent is running/u,
  );
  assert.deepEqual(busy.modeCalls, []);
  assert.deepEqual(busy.session.entries, []);
  assert.equal(busy.session.promptCalls.length, 0);

  const planless = deps();
  planless.plans.clear();
  await assert.rejects(
    executePlan("current", planless.dependencies),
    /No Plan is submitted/u,
  );
  assert.deepEqual(planless.modeCalls, []);
  assert.deepEqual(planless.session.entries, []);
  assert.equal(planless.session.promptCalls.length, 0);
});

test("fresh execution branches from the source session and adopts the same artifact", async () => {
  const ctx = deps();
  const sourceArtifact = ctx.plans.latest();

  const result = await executePlan("fresh", ctx.dependencies);
  const fresh = ctx.stubRuntime.session;

  assert.deepEqual(result, { sessionId: "session-2", context: "fresh" });
  assert.equal(ctx.stubRuntime.newSessionCalls, 1);
  assert.equal(
    ctx.stubRuntime.newSessionOptions?.parentSession,
    "/sessions/session-1.jsonl",
  );
  assert.deepEqual(ctx.session.entries, [
    { type: PLAN_MODE_ENTRY_TYPE, data: { active: false } },
  ]);
  assert.deepEqual(fresh.entries, [
    { type: PLAN_ENTRY_TYPE, data: sourceArtifact },
    { type: PLAN_MODE_ENTRY_TYPE, data: { active: false } },
  ]);
  assert.equal(fresh.promptCalls.length, 1);
  assert.match(fresh.promptCalls[0], /fresh session/u);
  assert.deepEqual(fresh.modelCalls, [fresh.model]);
  assert.deepEqual(fresh.thinkingCalls, ["high"]);
  assert.ok(
    ctx.sent.some(
      (message) =>
        message.type === "plan_state" &&
        (message.artifact as PlanArtifact).id === sourceArtifact?.id,
    ),
  );
  assert.deepEqual(ctx.plans.latest(), sourceArtifact);
});

test("fresh execution rejects oversized Plans before creating a session", async () => {
  const ctx = deps();
  ctx.session.model = { provider: "test", id: "model-small", contextWindow: 32_000 };
  ctx.dependencies.modelRuntime = {
    getModel: () => ctx.session.model,
  } as unknown as ModelRuntime;
  const huge = submittedArtifact({
    handoffMarkdown: "h".repeat(40_000),
  });
  ctx.plans.adopt(huge);
  const source = ctx.plans.latest();

  await assert.rejects(
    executePlan("fresh", ctx.dependencies),
    /shorten the Plan/u,
  );
  assert.equal(ctx.stubRuntime.newSessionCalls, 0);
  assert.deepEqual(ctx.modeCalls, []);
  assert.deepEqual(ctx.plans.latest(), source);
});

test("fresh execution falls back to the absolute budget when the window is unknown", async () => {
  const ctx = deps();
  ctx.session.model = {
    provider: "test",
    id: "model-unknown",
    contextWindow: null,
  };
  ctx.dependencies.modelRuntime = {
    getModel: () => ctx.session.model,
  } as unknown as ModelRuntime;

  const huge = submittedArtifact({
    handoffMarkdown: "h".repeat(100_000),
  });
  ctx.plans.adopt(huge);
  await assert.rejects(
    executePlan("fresh", ctx.dependencies),
    /shorten the Plan/u,
  );

  const fits = submittedArtifact({
    handoffMarkdown: "f".repeat(10_000),
  });
  ctx.plans.adopt(fits);
  await executePlan("fresh", ctx.dependencies);
  assert.equal(ctx.stubRuntime.newSessionCalls, 1);
});

test("fresh execution rejects an inherited model that cannot be used", async () => {
  const ctx = deps({ availableModel: undefined });
  ctx.dependencies.modelRuntime = {
    getModel: () => undefined,
  } as unknown as ModelRuntime;

  await assert.rejects(
    executePlan("fresh", ctx.dependencies),
    /unavailable in a new session/u,
  );
  assert.equal(ctx.stubRuntime.newSessionCalls, 0);
  assert.deepEqual(ctx.modeCalls, []);
});

test("cancelled fresh session creation fails without prompting", async () => {
  const ctx = deps();
  ctx.stubRuntime.newSessionCancelled = true;

  await assert.rejects(
    executePlan("fresh", ctx.dependencies),
    /cancelled/u,
  );
  assert.equal(ctx.session.promptCalls.length, 0);
});
