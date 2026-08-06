import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import type {
  AgentSessionRuntime,
  ModelRuntime,
} from "@earendil-works/pi-coding-agent";

import { ApprovalQueue, type ApprovalRequest } from "./approval.ts";
import { ContextBudgetManager } from "./context-manager.ts";
import type { HarnessConfig } from "./harness.ts";
import { HostBridge, PlanModeController } from "./main.ts";
import { PlanStore } from "./plan.ts";
import { QuestionQueue, type PlanQuestion } from "./questions.ts";
import { isJsonObject, type JsonObject } from "./protocol/validation.ts";

const HOST_COMMANDS = [
  "agents_reload",
  "agents_state",
  "approval_rule_revoke",
  "approval_rules",
  "approval_rules_clear",
  "approval_reply",
  "auth_cancel",
  "auth_list",
  "auth_login",
  "auth_logout",
  "auth_reply",
  "bootstrap_state",
  "context_state",
  "get_plan_state",
  "model_list",
  "model_set",
  "plan_execute",
  "queue_clear",
  "question_reply",
  "resource_reload",
  "resource_state",
  "session_browser_close",
  "session_browser_open",
  "session_browser_query",
  "session_new",
  "session_resume",
  "set_plan_mode",
  "subagent_cancel",
  "subagent_integrate",
  "subagent_start",
  "thinking_set",
  "tree_abort",
  "tree_copy",
  "tree_label",
  "tree_navigate",
  "tree_state",
  "workspace_trust",
].sort();

const HOST_EVENTS = [
  "agents_state",
  "auth_complete",
  "auth_notify",
  "auth_prompt",
  "auth_prompt_cancelled",
  "context_budget",
  "host_protocol_error",
  "host_warning",
  "plan_mode_state",
  "plan_ready",
  "plan_state",
  "question_cancelled",
  "question_request",
  "response",
  "session_list_progress",
  "subagent_integration",
  "subagent_state",
  "turn_timing",
  "workspace_state",
].sort();

class FakeSocket extends EventEmitter {
  destroyed = false;
  readonly written: string[] = [];

  setEncoding(_encoding: string): void {}

  write(chunk: string): boolean {
    this.written.push(chunk);
    return true;
  }

  destroy(): void {
    if (this.destroyed) return;
    this.destroyed = true;
    this.emit("close");
  }

  feed(chunk: string): void {
    this.emit("data", chunk);
  }
}

let socketCounter = 0;

function tempSocketPath(): string {
  socketCounter += 1;
  return join(tmpdir(), `nabla-baseline-${process.pid}-${socketCounter}.sock`);
}

function createBridge(options: {
  modelRuntime?: ModelRuntime;
  planMode?: PlanModeController;
  socketPath?: string;
} = {}): { bridge: HostBridge; socket: FakeSocket } {
  const planMode = options.planMode ?? new PlanModeController();
  const bridge = new HostBridge(
    options.socketPath ?? tempSocketPath(),
    options.modelRuntime ?? ({} as ModelRuntime),
    planMode,
    new PlanStore(),
    new ContextBudgetManager(),
    {
      schemaVersion: 2,
      maxParallel: 2,
      trustedWorkspaces: [],
      allowedProjectExtensions: [],
      profiles: {},
      diagnostics: [],
    } satisfies HarnessConfig,
    async () => undefined,
  );
  return { bridge, socket: new FakeSocket() };
}

function accept(bridge: HostBridge, socket: FakeSocket): void {
  (bridge as unknown as { accept(socket: FakeSocket): void }).accept(socket);
}

function messages(socket: FakeSocket): JsonObject[] {
  const parsed: JsonObject[] = [];
  for (const chunk of socket.written) {
    for (const line of chunk.split("\n")) {
      if (!line) continue;
      const value = JSON.parse(line) as unknown;
      if (isJsonObject(value)) parsed.push(value);
    }
  }
  return parsed;
}

function response(socket: FakeSocket, id: string): JsonObject {
  const message = messages(socket).find(
    (message) => message.id === id && message.type === "response",
  );
  assert.ok(message, `missing response for ${id}`);
  return message;
}

function fakeRuntime(isIdle: boolean, cwd = join(tmpdir(), "nabla-baseline-cwd")): AgentSessionRuntime {
  let activeTools: string[] = [];
  const session = {
    isIdle,
    sessionId: "session-1",
    sessionManager: {
      getCwd: () => cwd,
      getBranch: () => "main",
    },
    getActiveToolNames: () => activeTools,
    setActiveToolsByName: (names: string[]) => {
      activeTools = names;
    },
  };
  return { session } as unknown as AgentSessionRuntime;
}

const tick = () => new Promise<void>((resolve) => setImmediate(resolve));

test("host command and event inventories are stable", () => {
  const source = readFileSync(new URL("./main.ts", import.meta.url), "utf8");
  const switchStart = source.indexOf("switch (command) {");
  const switchEnd = source.indexOf("default:", switchStart);
  const commandSwitch = source.slice(switchStart, switchEnd);
  const commands = [...commandSwitch.matchAll(/case "([a-z0-9_]+)":/gu)]
    .map((match) => match[1])
    .sort();
  assert.deepEqual(commands, HOST_COMMANDS);

  const events = [...new Set(
    source
      .split(";")
      .filter((statement) => /this\.send/u.test(statement))
      .flatMap((statement) =>
        [...statement.matchAll(/type: "([a-z0-9_]+)"/gu)].map(
          (match) => match[1],
        ),
      ),
  )].sort();
  assert.deepEqual(events, HOST_EVENTS);

  const approvalSource = readFileSync(
    new URL("./approval.ts", import.meta.url),
    "utf8",
  );
  assert.match(approvalSource, /type: "approval_request"/u);
});

test("unknown command returns a failure response instead of crashing", async () => {
  const { bridge, socket } = createBridge();
  accept(bridge, socket);
  socket.feed('{"id":"r1","type":"no_such_command"}\n');
  await tick();
  const message = response(socket, "r1");
  assert.equal(message.command, "no_such_command");
  assert.equal(message.success, false);
  assert.equal(message.error, "Unknown host command");
});

test("invalid JSON and non-object JSON return protocol errors", async () => {
  const { bridge, socket } = createBridge();
  accept(bridge, socket);
  socket.feed("not json\n");
  await tick();
  assert.equal(messages(socket)[0]?.type, "host_protocol_error");

  socket.feed("[1,2]\n");
  await tick();
  const last = messages(socket).at(-1);
  assert.equal(last?.type, "response");
  assert.equal(last?.command, "unknown");
  assert.equal(last?.success, false);
});

test("connection close cancels the active authentication flow", async () => {
  let startedResolve!: () => void;
  const started = new Promise<void>((resolve) => {
    startedResolve = resolve;
  });
  let interaction!: Parameters<ModelRuntime["login"]>[2];
  const modelRuntime = {
    getProvider: (providerId: string) => ({
      id: providerId,
      name: "Fake Provider",
      auth: {
        apiKey: { name: "API key", login: async () => undefined },
      },
    }),
    login: async (
      _providerId: string,
      _authType: unknown,
      authInteraction: Parameters<ModelRuntime["login"]>[2],
    ) => {
      interaction = authInteraction;
      startedResolve();
      return new Promise<never>(() => {});
    },
  } as unknown as ModelRuntime;
  const { bridge, socket } = createBridge({ modelRuntime });
  accept(bridge, socket);
  socket.feed(
    '{"id":"a1","type":"auth_login","flowId":"flow-1","providerId":"fake","authType":"api_key"}\n',
  );
  await started;
  const prompt = interaction.prompt({ type: "text", message: "Enter key" } as never);
  await tick();
  assert.ok(
    socket.written.some((chunk) => chunk.includes('"auth_prompt"')),
    "auth prompt was announced",
  );
  socket.destroy();
  await assert.rejects(prompt, /Login cancelled/u);
});

test("connection close denies ordinary approvals and cancels questions", async () => {
  const { bridge, socket } = createBridge();
  accept(bridge, socket);
  const queues = bridge as unknown as {
    approvals: ApprovalQueue;
    questions: QuestionQueue;
  };
  const approval = queues.approvals.request(
    {
      requestId: "request-1",
      toolCallId: "tool-1",
      sessionId: "session-1",
      workspaceId: "workspace-1",
      summary: "Test approval",
      risk: "normal",
      intentDigest: "digest",
      availableDecisions: ["allow_once", "deny"],
      toolName: "bash",
      input: { command: "echo hi" },
    } satisfies ApprovalRequest,
    undefined,
    () => {},
  );
  const question = queues.questions.request(
    [
      {
        id: "q1",
        prompt: "Continue?",
        options: [
          { id: "yes", label: "Yes" },
          { id: "no", label: "No" },
        ],
      },
    ] satisfies PlanQuestion[],
    undefined,
    () => {},
    () => {},
  );
  socket.destroy();
  assert.equal(await approval, "deny");
  await assert.rejects(question, /disconnected/u);
});

test("connection close does not cancel running subagents", async () => {
  const { bridge, socket } = createBridge();
  accept(bridge, socket);
  const controller = new AbortController();
  const subagents = (
    bridge as unknown as { subagents: Map<string, { controller: AbortController }> }
  ).subagents;
  subagents.set("agent-1", { controller });
  socket.destroy();
  assert.equal(controller.signal.aborted, false);
  assert.equal(subagents.has("agent-1"), true);
});

test("session new/resume are rejected while the agent is running", async () => {
  const planMode = new PlanModeController();
  planMode.attach(fakeRuntime(false));
  const { bridge, socket } = createBridge({ planMode });
  accept(bridge, socket);
  socket.feed('{"id":"s1","type":"session_new"}\n');
  await tick();
  assert.equal(
    response(socket, "s1").error,
    "Cannot create a session while the agent is running",
  );
  socket.feed('{"id":"s2","type":"session_resume","sessionPath":"/x"}\n');
  await tick();
  assert.equal(
    response(socket, "s2").error,
    "Cannot resume a session while the agent is running",
  );
});

test("worktree recovery completes before the control socket listens", async () => {
  const planMode = new PlanModeController();
  const cwd = join(tmpdir(), "nabla-baseline-recovery");
  planMode.attach(fakeRuntime(true, cwd));
  const { bridge } = createBridge({ planMode });
  const order: string[] = [];
  let release!: () => void;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  (bridge as unknown as { worktrees: unknown }).worktrees = {
    listRecoverable: async () => {
      order.push("recovery");
      await gate;
      return { records: [], warnings: [] };
    },
    pruneTerminalArtifacts: async () => {
      order.push("prune");
    },
  };
  const listening = bridge.listen().then(() => order.push("listen"));
  await tick();
  assert.deepEqual(order, ["recovery"]);
  release();
  await listening;
  assert.deepEqual(order, ["recovery", "prune", "listen"]);
  await bridge.close();
});

test("plan_execute returns a failure response when no plan is submitted", async () => {
  const planMode = new PlanModeController();
  planMode.attach(fakeRuntime(true));
  const { bridge, socket } = createBridge({ planMode });
  accept(bridge, socket);
  socket.feed('{"id":"p1","type":"plan_execute","context":"current"}\n');
  await tick();
  assert.equal(response(socket, "p1").success, false);
  assert.equal(response(socket, "p1").error, "No Plan is submitted");
});
