import assert from "node:assert/strict";
import test from "node:test";

import type {
  AgentSession,
  AgentSessionRuntime,
  ModelRuntime,
} from "@earendil-works/pi-coding-agent";

import type { RuntimeAccess } from "../../runtime/runtime-access.ts";
import { ModelService } from "./model-service.ts";

function fakeSession(overrides: Partial<AgentSession> = {}): AgentSession {
  return {
    isIdle: true,
    sessionId: "session-1",
    model: { provider: "fake", id: "model-a" },
    setModel: async () => undefined,
    setThinkingLevel: () => undefined,
    thinkingLevel: "medium",
    getAvailableThinkingLevels: () => ["low", "medium", "high"],
    sessionManager: { getCwd: () => "/workspace" },
    getActiveToolNames: () => [],
    setActiveToolsByName: () => {},
    ...overrides,
  } as unknown as AgentSession;
}

function fakeRuntime(session: AgentSession): RuntimeAccess {
  return {
    current: () =>
      ({
        session,
        services: {
          settingsManager: {
            getDefaultProvider: () => "fake",
            getDefaultModel: () => "model-a",
          },
        },
      }) as unknown as AgentSessionRuntime,
    requireIdle: (operation: string) => {
      if (!session.isIdle) {
        throw new Error(`${operation} while the agent is running`);
      }
      return {
        session,
        services: {
          settingsManager: {
            getDefaultProvider: () => "fake",
            getDefaultModel: () => "model-a",
          },
        },
      } as unknown as AgentSessionRuntime;
    },
    sessionGeneration: () => 1,
  };
}

function fakeModelRuntime(overrides: Partial<ModelRuntime> = {}): ModelRuntime {
  return {
    getAvailable: async () => [
      {
        provider: "fake",
        id: "model-a",
        name: "Model A",
        reasoning: "high",
        contextWindow: 100_000,
      },
    ],
    getModel: (provider: string, id: string) =>
      provider === "fake" && id === "model-a"
        ? {
            provider,
            id,
            name: "Model A",
          }
        : undefined,
    ...overrides,
  } as unknown as ModelRuntime;
}

test("list returns the current model and the catalog", async () => {
  const service = new ModelService(
    fakeModelRuntime(),
    fakeRuntime(fakeSession()),
  );
  const snapshot = await service.list();
  assert.deepEqual(snapshot.current, { provider: "fake", id: "model-a" });
  assert.equal(snapshot.models.length, 1);
});

test("set rejects unknown models and busy sessions", async () => {
  const service = new ModelService(
    fakeModelRuntime(),
    fakeRuntime(fakeSession()),
  );
  await assert.rejects(
    service.set({ provider: "fake", modelId: "missing" }),
    /Unknown model: fake\/missing/u,
  );
  const busy = fakeRuntime(
    fakeSession({
      isIdle: false,
    }),
  );
  const busyService = new ModelService(fakeModelRuntime(), busy);
  await assert.rejects(
    busyService.set({ provider: "fake", modelId: "model-a" }),
    /Cannot change model while the agent is running/u,
  );
});

test("setThinking enforces busy state and returns compatibility info", () => {
  const service = new ModelService(
    fakeModelRuntime(),
    fakeRuntime(fakeSession()),
  );
  assert.deepEqual(service.setThinking("high"), {
    level: "medium",
    available: ["low", "medium", "high"],
  });
});

test("selectDefaultModel keeps the current model or picks the default", async () => {
  const service = new ModelService(
    fakeModelRuntime(),
    fakeRuntime(fakeSession()),
  );
  assert.deepEqual(await service.selectDefaultModel("fake"), {
    provider: "fake",
    id: "model-a",
  });

  let selected: unknown;
  const noModel = fakeRuntime(
    fakeSession({
      model: undefined,
      setModel: async (model: unknown) => {
        selected = model;
      },
    }),
  );
  const selecting = new ModelService(fakeModelRuntime(), noModel);
  await selecting.selectDefaultModel("fake");
  assert.ok(selected);
});

test("selectDefaultModel falls back safely on provider failures", async () => {
  const service = new ModelService(
    fakeModelRuntime({
      getAvailable: async () => {
        throw new Error("provider unavailable");
      },
    }),
    fakeRuntime(fakeSession({ model: undefined })),
  );
  assert.equal(await service.selectDefaultModel("fake"), undefined);
});
