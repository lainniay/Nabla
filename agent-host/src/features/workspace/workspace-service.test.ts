import assert from "node:assert/strict";
import test from "node:test";

import type {
  AgentSession,
  AgentSessionRuntime,
  ModelRuntime,
} from "@earendil-works/pi-coding-agent";

import type { HarnessConfig } from "../../harness.ts";
import type { RuntimeAccess } from "../../runtime/runtime-access.ts";
import { PlanModeService } from "../../runtime/plan-mode-service.ts";
import type { JsonObject } from "../../protocol/validation.ts";
import { WorkspaceService } from "./workspace-service.ts";

const config: HarnessConfig = {
  schemaVersion: 2,
  maxParallel: 2,
  trustedWorkspaces: [],
  allowedProjectExtensions: [],
  profiles: {
    worker: {
      description: "Worker profile",
      source: "builtin",
      skills: ["worker"],
      tools: ["read", "edit"],
      maxParallel: 1,
      maxTurns: 10,
      isolation: { mode: "auto", integration: "auto" },
      disabled: false,
      permission: {},
      instructions: [],
    },
  },
  diagnostics: [],
};

function fakeSession(): AgentSession {
  return {
    isIdle: true,
    sessionId: "session-1",
    sessionManager: { getCwd: () => "/workspace" },
    resourceLoader: {
      getSkills: () => ({
        skills: [{ name: "worker", filePath: "/agents/worker.md", description: "W" }],
        diagnostics: [],
      }),
      getPrompts: () => ({ prompts: [], diagnostics: [] }),
      getExtensions: () => ({
        extensions: [],
        errors: [],
        commands: new Map(),
      }),
      getAgentsFiles: () => ({ agentsFiles: [] }),
    },
    getActiveToolNames: () => [],
    setActiveToolsByName: () => {},
  } as unknown as AgentSession;
}

function fakeRuntime(session = fakeSession()): RuntimeAccess {
  return {
    current: () => ({ session }) as unknown as AgentSessionRuntime,
    requireIdle: () => ({ session }) as unknown as AgentSessionRuntime,
    sessionGeneration: () => 1,
  };
}

test("resource snapshot revision grows monotonically and publishes", () => {
  const events: JsonObject[] = [];
  const service = new WorkspaceService(
    fakeRuntime(),
    new PlanModeService(),
    {} as ModelRuntime,
    (event) => events.push(event),
    config,
  );
  const session = fakeSession();
  assert.equal(service.resourceSnapshot(session).revision, 1);
  const published = service.publishWorkspaceState(session, {
    revision: 0,
  } as never);
  assert.equal(published.resources.revision, 2);
  assert.equal(published.agents.revision, 0);
  assert.equal(events[0]?.type, "workspace_state");
});

test("profile catalog reports unavailable profiles and prompt text", () => {
  const service = new WorkspaceService(
    fakeRuntime(),
    new PlanModeService(),
    {} as ModelRuntime,
    () => {},
    config,
  );
  const session = fakeSession();
  const unavailable = service.profileUnavailableReason(config.profiles.worker!, session);
  assert.equal(unavailable, undefined);
  assert.match(service.subagentCatalogPrompt(), /worker: Worker profile/u);
});

test("reloadConfig replaces the active harness config", () => {
  const service = new WorkspaceService(
    fakeRuntime(),
    new PlanModeService(),
    {} as ModelRuntime,
    () => {},
    config,
  );
  service.reloadConfig("/elsewhere");
  assert.notEqual(service.configValue(), config);
  assert.equal(service.configValue().schemaVersion, 2);
  assert.ok(Array.isArray(service.configValue().trustedWorkspaces));
});
