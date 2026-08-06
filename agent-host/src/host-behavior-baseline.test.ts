import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { createConnection, type Socket } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import type {
  AgentSessionRuntime,
  ModelRuntime,
} from "@earendil-works/pi-coding-agent";

import type { ApprovalRequest } from "./approval.ts";
import type { PlanQuestion } from "./questions.ts";
import type { JsonObject } from "./protocol/validation.ts";
import type { InteractionBroker } from "./features/interactions/interaction-broker.ts";
import type { IntegrationService } from "./features/subagents/integration-service.ts";
import { RuntimeSupervisor } from "./runtime/runtime-supervisor.ts";
import type { HostApp } from "./app/host-app.ts";
import { createHostApp } from "./app/create-host-app.ts";

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

const EVENT_SOURCES = [
  "features/auth/auth-service.ts",
  "features/workspace/workspace-service.ts",
  "features/sessions/session-browser-service.ts",
  "features/context/context-service.ts",
  "features/plans/plan-service.ts",
  "features/subagents/subagent-supervisor.ts",
  "features/subagents/subagent-runner.ts",
  "runtime/pi-extension-factory.ts",
  "transport/control-connection.ts",
  "transport/control-server.ts",
  "protocol/command-router.ts",
];

class TestClient {
  private buffer = "";
  private readonly socket: Socket;
  readonly messages: JsonObject[] = [];
  private readonly closedPromise: Promise<void>;

  constructor(socket: Socket) {
    this.socket = socket;
    this.socket.setEncoding("utf8");
    this.socket.on("data", (chunk: string) => {
      this.buffer += chunk;
      while (true) {
        const newline = this.buffer.indexOf("\n");
        if (newline < 0) break;
        const line = this.buffer.slice(0, newline);
        this.buffer = this.buffer.slice(newline + 1);
        if (!line) continue;
        this.messages.push(JSON.parse(line) as JsonObject);
      }
    });
    this.closedPromise = new Promise((resolve) => socket.once("close", resolve));
  }

  write(data: string): void {
    this.socket.write(data);
  }

  close(): Promise<void> {
    this.socket.destroy();
    return this.closedPromise;
  }

  async waitFor(
    predicate: (message: JsonObject) => boolean,
    timeoutMs = 1_000,
  ): Promise<JsonObject> {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      const match = this.messages.find(predicate);
      if (match) return match;
      await new Promise((resolve) => setTimeout(resolve, 10));
    }
    throw new Error("timed out waiting for message");
  }
}

let socketCounter = 0;

function tempSocketPath(): string {
  socketCounter += 1;
  return join(tmpdir(), `nabla-baseline-${process.pid}-${socketCounter}.sock`);
}

async function createApp(options: {
  modelRuntime?: ModelRuntime;
  runtime?: RuntimeSupervisor;
  integrations?: IntegrationService;
} = {}): Promise<{ app: HostApp; socketPath: string; cwd: string }> {
  const cwd = mkdtempSync(join(tmpdir(), "nabla-baseline-app-"));
  const socketPath = tempSocketPath();
  const app = await createHostApp({
    socketPath,
    cwd,
    agentDir: join(cwd, "agents"),
    env: {},
    modelRuntime: options.modelRuntime ?? ({} as ModelRuntime),
    supervisor:
      options.runtime ??
      new RuntimeSupervisor(
        async () => {
          throw new Error("factory should not run");
        },
        fakeRuntime(true),
      ),
    integrations: options.integrations,
  });
  return { app, socketPath, cwd };
}

async function openClient(socketPath: string): Promise<TestClient> {
  return new Promise((resolve, reject) => {
    const socket = createConnection(socketPath);
    socket.once("connect", () => resolve(new TestClient(socket)));
    socket.once("error", reject);
  });
}

async function withBridge(
  options: {
    modelRuntime?: ModelRuntime;
    runtime?: RuntimeSupervisor;
    integrations?: IntegrationService;
  },
  run: (app: HostApp, client: TestClient) => Promise<void>,
): Promise<void> {
  const { app, socketPath, cwd } = await createApp(options);
  await app.start();
  const client = await openClient(socketPath);
  try {
    await run(app, client);
  } finally {
    await client.close();
    await app.close();
    rmSync(cwd, { recursive: true, force: true });
  }
}

function response(client: TestClient, id: string): Promise<JsonObject> {
  return client.waitFor(
    (message) => message.id === id && message.type === "response",
  );
}

function fakeRuntime(
  isIdle: boolean,
  cwd = join(tmpdir(), "nabla-baseline-cwd"),
): AgentSessionRuntime {
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
  return {
    session,
    dispose: async () => undefined,
  } as unknown as AgentSessionRuntime;
}

const tick = () => new Promise<void>((resolve) => setImmediate(resolve));

test("host command and event inventories are stable", () => {
  const commandSource = [
    "auth-commands.ts",
    "bootstrap-commands.ts",
    "configuration-commands.ts",
    "interaction-commands.ts",
    "model-commands.ts",
    "permission-commands.ts",
    "plan-commands.ts",
    "agent-commands.ts",
    "session-commands.ts",
  ]
    .map((file) =>
      readFileSync(new URL(`./protocol/commands/${file}`, import.meta.url), "utf8"),
    )
    .join("\n");
  const commands = [
    ...new Set(
      [...commandSource.matchAll(/type: "([a-z0-9_]+)"/gu)].map(
        (match) => match[1],
      ),
    ),
  ].sort();
  assert.deepEqual(commands, HOST_COMMANDS);

  const eventSource = EVENT_SOURCES.map((file) =>
    readFileSync(new URL(`./${file}`, import.meta.url), "utf8"),
  ).join("\n");
  const events = [
    ...new Set(
      [...eventSource.matchAll(/type: "([a-z0-9_]+)"/gu)]
        .map((match) => match[1])
        .filter(
          (name) =>
            name !== "api_key" &&
            name !== "oauth" &&
            name !== "error" &&
            name !== "text",
        ),
    ),
  ].sort();
  assert.deepEqual(events, HOST_EVENTS);

  const approvalSource = readFileSync(
    new URL("./approval.ts", import.meta.url),
    "utf8",
  );
  assert.match(approvalSource, /type: "approval_request"/u);
});

test("unknown command returns a failure response instead of crashing", async () => {
  await withBridge({}, async (_app, client) => {
    client.write('{"id":"r1","type":"no_such_command"}\n');
    const message = await response(client, "r1");
    assert.equal(message.command, "no_such_command");
    assert.equal(message.success, false);
    assert.equal(message.error, "Unknown host command");
  });
});

test("invalid JSON and non-object JSON return protocol errors", async () => {
  await withBridge({}, async (_app, client) => {
    client.write("not json\n");
    const parseError = await client.waitFor(
      (m) => m.type === "host_protocol_error",
    );
    assert.ok(String(parseError.error).length > 0);

    client.write("[1,2]\n");
    const requestError = await client.waitFor(
      (m) =>
        m.type === "host_protocol_error" &&
        String(m.error).includes("must be a JSON object"),
    );
    assert.ok(String(requestError.error).includes("Host request"));

    client.write('{"id":"r2","type":"no_such_command"}\n');
    assert.equal((await response(client, "r2")).success, false);
  });
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
  await withBridge({ modelRuntime }, async (_app, client) => {
    client.write(
      '{"id":"a1","type":"auth_login","flowId":"flow-1","providerId":"fake","authType":"api_key"}\n',
    );
    await started;
    const prompt = interaction.prompt({ type: "text", message: "Enter key" } as never);
    await client.waitFor((m) => m.type === "auth_prompt");
    const promptRejected = assert.rejects(prompt, /Login cancelled/u);
    await client.close();
    await promptRejected;
  });
});

test("connection close denies ordinary approvals and cancels questions", async () => {
  await withBridge({}, async (app, client) => {
    const interactions = (
      app as unknown as { interactions: InteractionBroker }
    ).interactions;
    const approval = interactions.requestApproval(
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
    const question = interactions.requestQuestions(
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
    await client.close();
    assert.equal(await approval, "deny");
    await assert.rejects(question, /disconnected/u);
  });
});

test("connection close does not cancel running subagents", async () => {
  await withBridge({}, async (app, client) => {
    let hostClosed = 0;
    const subagents = (
      app as unknown as { subagents: { hostClose: () => Promise<void> } }
    ).subagents;
    subagents.hostClose = async () => {
      hostClosed += 1;
    };
    await client.close();
    assert.equal(hostClosed, 0);
  });
});

test("session new/resume are rejected while the agent is running", async () => {
  const runtime = new RuntimeSupervisor(
    async () => {
      throw new Error("factory should not run");
    },
    fakeRuntime(false),
  );
  await withBridge({ runtime }, async (_app, client) => {
    client.write('{"id":"s1","type":"session_new"}\n');
    assert.equal(
      (await response(client, "s1")).error,
      "Cannot create a session while the agent is running",
    );
    client.write('{"id":"s2","type":"session_resume","sessionPath":"/x"}\n');
    assert.equal(
      (await response(client, "s2")).error,
      "Cannot resume a session while the agent is running",
    );
  });
});

test("worktree recovery completes before the control socket listens", async () => {
  const cwd = join(tmpdir(), "nabla-baseline-recovery");
  const runtime = new RuntimeSupervisor(
    async () => {
      throw new Error("factory should not run");
    },
    fakeRuntime(true, cwd),
  );
  const order: string[] = [];
  let release!: () => void;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  const integrations = {
    recover: async () => {
      order.push("recovery");
      await gate;
      order.push("prune");
      return [];
    },
  } as unknown as IntegrationService;
  const { app, socketPath } = await createApp({ runtime, integrations });
  const starting = app.start().then(() => order.push("listen"));
  await tick();
  assert.deepEqual(order, ["recovery"]);
  release();
  await starting;
  assert.deepEqual(order, ["recovery", "prune", "listen"]);
  await app.close();
});

test("plan_execute returns a failure response when no plan is submitted", async () => {
  const runtime = new RuntimeSupervisor(
    async () => {
      throw new Error("factory should not run");
    },
    fakeRuntime(true),
  );
  await withBridge({ runtime }, async (_app, client) => {
    client.write('{"id":"p1","type":"plan_execute","context":"current"}\n');
    const message = await response(client, "p1");
    assert.equal(message.success, false);
    assert.equal(message.error, "No Plan is submitted");
  });
});
