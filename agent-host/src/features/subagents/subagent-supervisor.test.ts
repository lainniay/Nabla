import assert from "node:assert/strict";
import test from "node:test";

import type { ModelRuntime } from "@earendil-works/pi-coding-agent";

import type { HarnessConfig } from "../../harness.ts";
import type { RuntimeSupervisor } from "../../runtime/runtime-supervisor.ts";
import { PlanModeService } from "../../runtime/plan-mode-service.ts";
import type { JsonObject } from "../../protocol/validation.ts";
import type { WorkspaceService } from "../workspace/workspace-service.ts";
import type { PermissionService } from "../permissions/permission-service.ts";
import type { IntegrationService } from "./integration-service.ts";
import { SubagentSupervisor } from "./subagent-supervisor.ts";

const config: HarnessConfig = {
  schemaVersion: 2,
  maxParallel: 2,
  trustedWorkspaces: [],
  allowedProjectExtensions: [],
  profiles: {
    worker: {
      description: "Worker",
      source: "builtin",
      skills: [],
      tools: ["read"],
      permission: {},
      maxParallel: 1,
      maxTurns: 10,
      isolation: { mode: "none", integration: "auto" },
      disabled: false,
      instructions: [],
    },
  },
  diagnostics: [],
};

function build() {
  const events: JsonObject[] = [];
  let agentsChanged = 0;
  let releasePrepare!: () => void;
  const prepareGate = new Promise<void>((resolve) => {
    releasePrepare = resolve;
  });
  const supervisor = new SubagentSupervisor(
    {
      configValue: () => config,
      profileUnavailableReason: () => undefined,
    } as unknown as WorkspaceService,
    {
      prepare: async () => {
        await prepareGate;
        return {
          backend: "shared",
          executionCwd: "/workspace",
          warning: undefined,
          record: undefined,
        };
      },
      annotate: async (record: never) => record,
    } as unknown as IntegrationService,
    {} as PermissionService,
    {
      getModel: (provider: string, id: string) =>
        provider === "fake" && id === "model-a"
          ? { provider, id, name: "Model A" }
          : undefined,
    } as unknown as ModelRuntime,
    {
      current: () => ({
        session: {
          sessionId: "session-1",
          model: { provider: "fake", id: "model-a" },
          sessionManager: { getCwd: () => "/workspace" },
        },
      }),
    } as unknown as RuntimeSupervisor,
    new PlanModeService(),
    (event) => events.push(event),
    () => {},
    () => {
      agentsChanged += 1;
    },
  );
  return { supervisor, events, releasePrepare, agentsChanged: () => agentsChanged };
}

test("start rejects unknown and disabled profiles", () => {
  const { supervisor } = build();
  assert.throws(
    () => supervisor.start({ profile: "missing", task: "work" }),
    /Unknown agent profile: missing/u,
  );
  const disabled = structuredClone(config);
  disabled.profiles.worker!.disabled = true;
  const withDisabled = new SubagentSupervisor(
    { configValue: () => disabled } as unknown as WorkspaceService,
    {} as IntegrationService,
    {} as PermissionService,
    {} as ModelRuntime,
    {} as RuntimeSupervisor,
    new PlanModeService(),
    () => {},
    () => {},
    () => {},
  );
  assert.throws(
    () => withDisabled.start({ profile: "worker", task: "work" }),
    /Subagent profile is disabled/u,
  );
});

test("start queues a subagent and publishes state", () => {
  const { supervisor, events, agentsChanged } = build();
  const result = supervisor.start({ profile: "worker", task: "work" });
  assert.equal(result.accepted, true);
  assert.equal(result.agent.id, "agent-1");
  assert.ok(events.some((event) => event.type === "subagent_state"));
  assert.equal(supervisor.activeSnapshots().length, 1);
  assert.ok(agentsChanged() > 0);
});

test("concurrency limits reject additional starts", () => {
  const { supervisor } = build();
  supervisor.start({ profile: "worker", task: "first" });
  const limited = structuredClone(config);
  limited.maxParallel = 1;
  const single = new SubagentSupervisor(
    {
      configValue: () => limited,
      profileUnavailableReason: () => undefined,
    } as unknown as WorkspaceService,
    {} as IntegrationService,
    {} as PermissionService,
    {
      getModel: () => ({ provider: "fake", id: "model-a" }),
    } as unknown as ModelRuntime,
    {
      current: () => ({
        session: {
          sessionId: "session-1",
          model: { provider: "fake", id: "model-a" },
          sessionManager: { getCwd: () => "/workspace" },
        },
      }),
    } as unknown as RuntimeSupervisor,
    new PlanModeService(),
    () => {},
    () => {},
    () => {},
  );
  single.start({ profile: "worker", task: "first" });
  assert.throws(
    () => single.start({ profile: "worker", task: "second" }),
    /Subagent concurrency limit reached \(1\)/u,
  );
});

test("pre-aborted parent cancels the queued subagent", async () => {
  const { supervisor, events, releasePrepare } = build();
  const parent = new AbortController();
  parent.abort();
  const completion = supervisor.run({
    task: "work",
    profile: "worker",
    parentSignal: parent.signal,
  });
  releasePrepare();
  await assert.rejects(completion, /Subagent cancelled/u);
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(supervisor.activeSnapshots().length, 0);
  assert.ok(
    events.some(
      (event) =>
        event.type === "subagent_state" && event.event === "cancelled",
    ),
  );
});

test("hostClose cancels all running subagents", async () => {
  const { supervisor, releasePrepare } = build();
  const completion = supervisor.run({
    task: "work",
    profile: "worker",
  });
  await supervisor.hostClose();
  releasePrepare();
  await assert.rejects(completion, /Subagent cancelled/u);
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(supervisor.activeSnapshots().length, 0);
});

test("subagent ids never repeat", () => {
  const parallel = structuredClone(config);
  parallel.profiles.worker!.maxParallel = 2;
  const { supervisor } = buildWithConfig(parallel);
  const first = supervisor.start({ profile: "worker", task: "a" });
  const second = supervisor.start({ profile: "worker", task: "b" });
  assert.notEqual(first.agent.id, second.agent.id);
});

function buildWithConfig(harness: HarnessConfig) {
  const events: JsonObject[] = [];
  let releasePrepare!: () => void;
  const prepareGate = new Promise<void>((resolve) => {
    releasePrepare = resolve;
  });
  const supervisor = new SubagentSupervisor(
    {
      configValue: () => harness,
      profileUnavailableReason: () => undefined,
    } as unknown as WorkspaceService,
    {
      prepare: async () => {
        await prepareGate;
        return {
          backend: "shared",
          executionCwd: "/workspace",
          warning: undefined,
          record: undefined,
        };
      },
      annotate: async (record: never) => record,
    } as unknown as IntegrationService,
    {} as PermissionService,
    {
      getModel: (provider: string, id: string) =>
        provider === "fake" && id === "model-a"
          ? { provider, id, name: "Model A" }
          : undefined,
    } as unknown as ModelRuntime,
    {
      current: () => ({
        session: {
          sessionId: "session-1",
          model: { provider: "fake", id: "model-a" },
          sessionManager: { getCwd: () => "/workspace" },
        },
      }),
    } as unknown as RuntimeSupervisor,
    new PlanModeService(),
    (event) => events.push(event),
    () => {},
    () => {},
  );
  return { supervisor, events, releasePrepare };
}
