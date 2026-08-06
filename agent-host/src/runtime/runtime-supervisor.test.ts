import assert from "node:assert/strict";
import test from "node:test";

import type {
  AgentSession,
  AgentSessionRuntime,
  CreateAgentSessionRuntimeResult,
} from "@earendil-works/pi-coding-agent";

import { RuntimeSupervisor } from "./runtime-supervisor.ts";

function fakeSession(isIdle = true): AgentSession {
  return {
    isIdle,
    sessionId: "session-1",
    sessionManager: { getCwd: () => "/workspace" },
    getActiveToolNames: () => [],
    setActiveToolsByName: () => {},
    extensionRunner: {
      hasHandlers: () => false,
      emit: async () => undefined,
    },
    dispose: () => {},
  } as unknown as AgentSession;
}

function fakeRuntime(overrides: Partial<AgentSessionRuntime> = {}): AgentSessionRuntime {
  return {
    session: fakeSession(),
    dispose: async () => undefined,
    newSession: async () => ({ cancelled: false }),
    switchSession: async () => ({ cancelled: false }),
    ...overrides,
  } as unknown as AgentSessionRuntime;
}

test("current() throws before initialization", () => {
  const supervisor = new RuntimeSupervisor(async () => {
    throw new Error("unused");
  });
  assert.throws(() => supervisor.current(), /Agent runtime is not ready/u);
  assert.throws(
    () => supervisor.requireIdle("Cannot reload resources"),
    /Agent runtime is not ready/u,
  );
});

test("requireIdle rejects busy sessions with the operation prefix", () => {
  const supervisor = new RuntimeSupervisor(
    async () => {
      throw new Error("unused");
    },
    fakeRuntime({ session: fakeSession(false) }),
  );
  assert.throws(
    () => supervisor.requireIdle("Cannot create a session"),
    /Cannot create a session while the agent is running/u,
  );
});

test("successful transitions bump generation and cancelled transitions do not", async () => {
  let cancelled = false;
  const runtime = fakeRuntime({
    newSession: async () => ({ cancelled }),
    switchSession: async () => ({ cancelled }),
  });
  const supervisor = new RuntimeSupervisor(
    async () => {
      throw new Error("unused");
    },
    runtime,
  );
  assert.equal(supervisor.sessionGeneration(), 1);
  await supervisor.newSession();
  assert.equal(supervisor.sessionGeneration(), 2);
  await supervisor.switchSession("/other");
  assert.equal(supervisor.sessionGeneration(), 3);
  cancelled = true;
  await supervisor.newSession();
  await supervisor.switchSession("/other");
  assert.equal(supervisor.sessionGeneration(), 3);
});

test("close clears the runtime and is idempotent", async () => {
  let disposed = 0;
  const supervisor = new RuntimeSupervisor(
    async () => {
      throw new Error("unused");
    },
    fakeRuntime({
      dispose: async () => {
        disposed += 1;
      },
    }),
  );
  await supervisor.close();
  await supervisor.close();
  assert.equal(disposed, 1);
  assert.throws(() => supervisor.current(), /Agent runtime is not ready/u);
});

test("initialize creates the runtime through the factory", async () => {
  const supervisor = new RuntimeSupervisor(
    async () =>
      ({
        session: fakeSession(),
        services: {},
        diagnostics: [],
      }) as unknown as CreateAgentSessionRuntimeResult,
  );
  const runtime = await supervisor.initialize({
    cwd: "/workspace",
    agentDir: "/agents",
    sessionManager: {
      getSessionFile: () => undefined,
      getCwd: () => "/workspace",
    } as never,
  });
  assert.equal(runtime.session.sessionId, "session-1");
  assert.equal(supervisor.sessionGeneration(), 1);
  await supervisor.close();
});
