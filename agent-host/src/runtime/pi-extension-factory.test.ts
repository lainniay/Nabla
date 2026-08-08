import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import type { JsonObject } from "../protocol/validation.ts";
import { TODO_ENTRY_TYPE, TodoStore } from "../features/todos/store.ts";
import {
  PiExtensionFactory,
  type PiExtensionPort,
} from "./pi-extension-factory.ts";

class FakePi {
  readonly tools: Array<{ name: string; execute: (...args: unknown[]) => unknown }> = [];
  readonly handlers = new Map<string, (...args: unknown[]) => unknown>();
  readonly entries: Array<{ type: string; entry: unknown }> = [];

  registerTool(tool: { name: string; execute: (...args: unknown[]) => unknown }): void {
    this.tools.push(tool);
  }

  on(event: string, handler: (...args: unknown[]) => unknown): void {
    this.handlers.set(event, handler);
  }

  appendEntry(type: string, entry: unknown): void {
    this.entries.push({ type, entry });
  }
}

function createFactory(overrides: Partial<PiExtensionPort> = {}) {
  const calls: string[] = [];
  const events: JsonObject[] = [];
  const port = {
    planMode: {
      current: () => false,
      set: () => ({ active: false, activeTools: [] }),
    },
    plans: {
      submit: () => {
        calls.push("plans.submit");
        return { id: "plan-1", revision: 1 };
      },
      snapshot: () => null,
      onSessionActivated: () => {
        calls.push("plans.onSessionActivated");
        return null;
      },
    },
    todos: {
      replace: () => {
        calls.push("todos.replace");
        return { action: "updated", todos: [] };
      },
      onSessionActivated: () => {
        calls.push("todos.onSessionActivated");
        return [];
      },
    },
    context: {
      snapshot: () => ({ revision: 1 }),
      onRuntimeSessionStart: () => {
        calls.push("context.onRuntimeSessionStart");
      },
      filter: () => {
        calls.push("context.filter");
        return { messages: [], snapshot: { revision: 1 } };
      },
      onModelResponse: () => {
        calls.push("context.onModelResponse");
        return { revision: 1 };
      },
      onCompaction: () => {
        calls.push("context.onCompaction");
        return { revision: 1 };
      },
      publish: () => {
        calls.push("context.publish");
      },
    },
    interactions: {
      requestQuestions: async () => {
        calls.push("interactions.requestQuestions");
        return [{ questionId: "q1", value: "yes" }];
      },
    },
    subagents: {
      run: async () => {
        calls.push("subagents.run");
        return { status: "completed" };
      },
    },
    permissions: {
      authorizeTool: async () => {
        calls.push("permissions.authorizeTool");
        return {
          permit: {
            id: "p1",
            toolCallId: "t1",
            intentDigest: "d",
            sandboxProfile: null,
          },
        };
      },
      finishTool: (_permit: unknown) => {
        calls.push("permissions.finishTool");
      },
    },
    workspace: {
      subagentCatalogPrompt: () => "profiles",
    },
    send: (event: JsonObject) => events.push(event),
    ...overrides,
  } as unknown as PiExtensionPort;
  const factory = new PiExtensionFactory(port);
  const pi = new FakePi();
  (factory.create() as { factory: (pi: never) => void }).factory(pi as never);
  return { calls, events, pi };
}

test("registers the control tools", () => {
  const { pi } = createFactory();
  assert.deepEqual(
    pi.tools.map((tool) => tool.name).sort(),
    ["ask_user", "delegate_task", "submit_plan", "todo_write"],
  );
});

test("todo_write replaces the list and appends a nabla.todo entry", async () => {
  const { pi } = createFactory({ todos: new TodoStore() });
  const tool = pi.tools.find((tool) => tool.name === "todo_write")!;
  const result = (await tool.execute(
    "t1",
    {
      todos: [
        { content: "Build", status: "in_progress" },
        { content: "Test", status: "pending" },
      ],
    },
    undefined,
  )) as { content: Array<{ text: string }> };
  assert.ok(result.content[0].text.includes('"action":"created"'));
  assert.ok(result.content[0].text.includes('"in_progress"'));
  assert.equal(pi.entries.length, 1);
  assert.equal(pi.entries[0].type, TODO_ENTRY_TYPE);
  assert.deepEqual(pi.entries[0].entry, [
    { content: "Build", status: "in_progress" },
    { content: "Test", status: "pending" },
  ]);
});

test("todo_write rejects invalid lists without appending", async () => {
  const { pi } = createFactory({ todos: new TodoStore() });
  const tool = pi.tools.find((tool) => tool.name === "todo_write")!;
  await assert.rejects(
    tool.execute(
      "t1",
      {
        todos: [
          { content: "a", status: "in_progress" },
          { content: "b", status: "in_progress" },
        ],
      },
      undefined,
    ) as Promise<unknown>,
    /at most one/u,
  );
  assert.equal(pi.entries.length, 0);
});

test("todo_write with an empty list clears and appends an empty entry", async () => {
  const { pi } = createFactory({ todos: new TodoStore() });
  const tool = pi.tools.find((tool) => tool.name === "todo_write")!;
  const result = (await tool.execute("t1", { todos: [] }, undefined)) as {
    content: Array<{ text: string }>;
  };
  assert.equal(result.content[0].text, '{"action":"created","todos":[]}');
  assert.deepEqual(pi.entries, [{ type: TODO_ENTRY_TYPE, entry: [] }]);
});

test("session_start restores the todo list from the session branch", () => {
  const store = new TodoStore();
  const { pi } = createFactory({ todos: store });
  pi.handlers.get("session_start")?.(
    {},
    {
      sessionManager: {
        getSessionId: () => "s1",
        getBranch: () => [
          {
            type: "custom",
            customType: TODO_ENTRY_TYPE,
            data: [{ content: "Build", status: "in_progress" }],
          },
        ],
      },
      getContextUsage: () => undefined,
    },
  );
  assert.deepEqual(store.current(), [
    { content: "Build", status: "in_progress" },
  ]);
});

test("submit_plan is rejected outside plan mode", async () => {
  const { pi } = createFactory();
  const submit = pi.tools.find((tool) => tool.name === "submit_plan")!;
  await assert.rejects(
    submit.execute("t1", { title: "x" }, undefined, undefined, {
      sessionManager: { getSessionId: () => "s1" },
    }) as Promise<unknown>,
    /only available in Plan mode/u,
  );
});

test("submit_plan submits, appends, and emits plan_ready in plan mode", async () => {
  const { calls, events, pi } = createFactory({
    planMode: {
      current: () => true,
      set: () => ({ active: true, activeTools: [] }),
    },
  });
  const submit = pi.tools.find((tool) => tool.name === "submit_plan")!;
  const result = (await submit.execute(
    "t1",
    {
      title: "Plan",
      summary: "S",
      bodyMarkdown: "B",
      assumptions: [],
      testPlan: [],
      handoffMarkdown: "H",
    },
    undefined,
    undefined,
    { sessionManager: { getSessionId: () => "s1" } },
  )) as { terminate: boolean };
  assert.equal(result.terminate, true);
  assert.ok(calls.includes("plans.submit"));
  assert.equal(pi.entries.length, 1);
  assert.ok(events.some((event) => event.type === "plan_ready"));
});

test("ask_user and delegate_task call their service ports", async () => {
  const { calls, pi } = createFactory();
  const ask = pi.tools.find((tool) => tool.name === "ask_user")!;
  const delegated = pi.tools.find((tool) => tool.name === "delegate_task")!;
  await ask.execute("t1", { questions: [] }, undefined);
  await delegated.execute("t1", { task: "work" }, undefined);
  assert.ok(calls.includes("interactions.requestQuestions"));
  assert.ok(calls.includes("subagents.run"));
});

test("session, context, and permission hooks route to service ports", async () => {
  const { calls, pi } = createFactory();
  pi.handlers.get("session_start")?.(
    {},
    {
      sessionManager: { getSessionId: () => "s1", getBranch: () => [] },
      getContextUsage: () => undefined,
    },
  );
  pi.handlers.get("context")?.(
    { messages: [] },
    { getContextUsage: () => undefined },
  );
  await pi.handlers.get("tool_call")?.(
    { toolCallId: "t1", toolName: "read", input: { path: "a.ts" } },
    { cwd: "/workspace", signal: undefined },
  );
  pi.handlers.get("tool_result")?.(
    { toolCallId: "t1", toolName: "read", isError: false, input: { path: "a.ts" } },
  );
  assert.ok(calls.includes("context.onRuntimeSessionStart"));
  assert.ok(calls.includes("plans.onSessionActivated"));
  assert.ok(calls.includes("todos.onSessionActivated"));
  assert.ok(calls.includes("context.filter"));
  assert.ok(calls.includes("context.publish"));
  assert.ok(calls.includes("permissions.authorizeTool"));
  assert.ok(calls.includes("permissions.finishTool"));
});

test("turn timing finalizes only on agent_settled", () => {
  const { events, pi } = createFactory();
  pi.handlers.get("agent_start")?.();
  pi.handlers.get("agent_end")?.();
  assert.ok(events.some((event) => event.type === "turn_timing" && event.phase === "started"));
  assert.ok(!events.some((event) => event.type === "turn_timing" && event.phase === "completed"));
  assert.equal(pi.entries.length, 0);

  pi.handlers.get("agent_settled")?.();
  assert.ok(events.some((event) => event.type === "turn_timing" && event.phase === "completed"));
  assert.equal(pi.entries.length, 1);
});

test("retry runs share one turn timing finalized at agent_settled", () => {
  const { events, pi } = createFactory();
  pi.handlers.get("agent_start")?.();
  pi.handlers.get("agent_end")?.();
  pi.handlers.get("agent_start")?.();
  pi.handlers.get("agent_end")?.();
  assert.equal(pi.entries.length, 0);

  pi.handlers.get("agent_settled")?.();
  assert.equal(pi.entries.length, 1);
  assert.equal(
    events.filter((event) => event.type === "turn_timing" && event.phase === "completed").length,
    1,
  );
});

test("before_agent_start augments the system prompt", () => {
  const root = mkdtempSync(join(tmpdir(), "nabla-extension-context-"));
  try {
    writeFileSync(join(root, "a.ts"), "x");
    const { pi } = createFactory();
    const result = pi.handlers
      .get("before_agent_start")
      ?.({ systemPrompt: "base" }, { cwd: root }) as
      | { systemPrompt?: string }
      | undefined;
    assert.match(String(result?.systemPrompt), /Follow Pi's normal interactive agent behavior/u);
    assert.match(String(result?.systemPrompt), /profiles/u);
    assert.match(
      String(result?.systemPrompt),
      new RegExp(`Current working directory: ${root.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&")}`),
    );
    assert.match(String(result?.systemPrompt), /Working directory tree:/u);
    assert.match(String(result?.systemPrompt), /- a\.ts/u);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("tool_call normalizes workspace absolute paths to relative", () => {
  const root = mkdtempSync(join(tmpdir(), "nabla-extension-normalize-"));
  try {
    const { calls, pi } = createFactory();
    const input = { path: join(root, "a.ts") };
    pi.handlers.get("tool_call")?.(
      { toolCallId: "t1", toolName: "read", input },
      { cwd: root, signal: undefined },
    );
    assert.equal(input.path, "a.ts");
    assert.ok(calls.includes("permissions.authorizeTool"));
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
