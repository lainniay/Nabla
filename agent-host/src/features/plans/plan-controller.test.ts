import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { ModelRuntime, SessionManager } from "@earendil-works/pi-coding-agent";

import type { RuntimeAccess } from "../../runtime/runtime-access.ts";
import type { PermissionIntent } from "../permissions/model.ts";
import { evaluatePermission } from "../permissions/evaluator.ts";
import type { JsonObject } from "../../protocol/validation.ts";
import {
  PLAN_MODE_POLICY,
  PlanController,
} from "./plan-controller.ts";
import {
  PLAN_ENTRY_TYPE,
  PLAN_MODE_ENTRY_TYPE,
  type PlanArtifact,
  type PlanContent,
} from "./model.ts";
import { PlanStore } from "./store.ts";

const content: PlanContent = {
  title: "Plan",
  summary: "Summary",
  bodyMarkdown: "Body",
  assumptions: [],
  testPlan: [],
  handoffMarkdown: "Handoff",
};

function fakeRuntime(session: {
  isIdle: boolean;
  sessionId: string;
  sessionManager: SessionManager;
  getActiveToolNames(): string[];
  setActiveToolsByName(names: string[]): void;
  prompt(): Promise<void>;
}): RuntimeAccess {
  return {
    current: () => ({ session }) as never,
    requireIdle: () => {
      if (!session.isIdle) {
        throw new Error("Cannot switch mode while the agent is running");
      }
      return { session } as never;
    },
    sessionGeneration: () => 1,
  };
}

test("restore applies the expected tools and tracks active state", () => {
  const root = mkdtempSync(join(tmpdir(), "nabla-plan-controller-"));
  try {
    const manager = SessionManager.create(root, root);
    const activeTools: string[] = [];
    const session = {
      isIdle: true,
      sessionId: manager.getSessionId(),
      sessionManager: manager,
      getActiveToolNames: () => activeTools,
      setActiveToolsByName: (names: string[]) => {
        activeTools.length = 0;
        activeTools.push(...names);
      },
      prompt: async () => undefined,
    };
    const controller = new PlanController(
      new PlanStore(),
      {} as ModelRuntime,
      fakeRuntime(session),
      () => {},
    );
    assert.equal(controller.current(), false);
    manager.appendCustomEntry(PLAN_MODE_ENTRY_TYPE, { active: true });
    controller.activateSession(manager.getBranch());
    assert.equal(controller.current(), true);
    assert.deepEqual(activeTools, PLAN_MODE_POLICY.exposedTools);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("set rejects busy sessions without changing mode", () => {
  const root = mkdtempSync(join(tmpdir(), "nabla-plan-controller-"));
  try {
    const manager = SessionManager.create(root, root);
    const session = {
      isIdle: false,
      sessionId: manager.getSessionId(),
      sessionManager: manager,
      getActiveToolNames: () => [],
      setActiveToolsByName: () => {},
      prompt: async () => undefined,
    };
    const controller = new PlanController(
      new PlanStore(),
      {} as ModelRuntime,
      fakeRuntime(session),
      () => {},
    );
    assert.throws(
      () => controller.setMode(true),
      /Cannot switch mode while the agent is running/u,
    );
    assert.equal(controller.current(), false);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("missing tools roll back to the previous tool set", () => {
  const root = mkdtempSync(join(tmpdir(), "nabla-plan-controller-"));
  try {
    const manager = SessionManager.create(root, root);
    const attempts: string[][] = [];
    const session = {
      isIdle: true,
      sessionId: manager.getSessionId(),
      sessionManager: manager,
      getActiveToolNames: () => [],
      setActiveToolsByName: (names: string[]) => {
        attempts.push(names);
      },
      prompt: async () => undefined,
    };
    const controller = new PlanController(
      new PlanStore(),
      {} as ModelRuntime,
      fakeRuntime(session),
      () => {},
    );
    assert.throws(
      () => controller.setMode(true),
      /Pi did not register required tools/u,
    );
    assert.deepEqual(attempts, [
      [...PLAN_MODE_POLICY.exposedTools],
      [],
    ]);
    assert.equal(controller.current(), false);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("submit bumps revision and snapshot exposes the artifact", () => {
  const root = mkdtempSync(join(tmpdir(), "nabla-plan-controller-"));
  try {
    const manager = SessionManager.create(root, root);
    const session = {
      isIdle: true,
      sessionId: manager.getSessionId(),
      sessionManager: manager,
      getActiveToolNames: () => [],
      setActiveToolsByName: () => {},
      prompt: async () => undefined,
    };
    const controller = new PlanController(
      new PlanStore(),
      {} as ModelRuntime,
      fakeRuntime(session),
      () => {},
    );
    const first = controller.submit(content, manager.getSessionId());
    const second = controller.submit(content, manager.getSessionId());
    assert.equal(first.revision, 1);
    assert.equal(second.revision, 2);
    assert.equal(controller.snapshot()?.id, second.id);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("activateSession restores the plan entry and publishes state", () => {
  const root = mkdtempSync(join(tmpdir(), "nabla-plan-controller-"));
  try {
    const manager = SessionManager.create(root, root);
    const artifact: PlanArtifact = {
      ...content,
      id: "plan-1",
      revision: 1,
      sourceSessionId: manager.getSessionId(),
      createdAt: "2026-01-01T00:00:00.000Z",
      updatedAt: "2026-01-01T00:00:00.000Z",
    };
    manager.appendCustomEntry(PLAN_ENTRY_TYPE, artifact);
    const session = {
      isIdle: true,
      sessionId: manager.getSessionId(),
      sessionManager: manager,
      getActiveToolNames: () => [],
      setActiveToolsByName: () => {},
      prompt: async () => undefined,
    };
    const events: JsonObject[] = [];
    const controller = new PlanController(
      new PlanStore(),
      {} as ModelRuntime,
      fakeRuntime(session),
      (event) => events.push(event),
    );
    controller.activateSession(manager.getBranch());
    assert.deepEqual(events[0]?.artifact, artifact);
    assert.ok(events.some((event) => event.type === "plan_mode_state"));
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("current execute starts a normal turn and exits plan mode", async () => {
  const root = mkdtempSync(join(tmpdir(), "nabla-plan-controller-"));
  try {
    const manager = SessionManager.create(root, root);
    manager.appendCustomEntry(PLAN_MODE_ENTRY_TYPE, { active: true });
    const activeTools: string[] = [];
    let prompted = 0;
    const session = {
      isIdle: true,
      sessionId: manager.getSessionId(),
      sessionManager: manager,
      getActiveToolNames: () => activeTools,
      setActiveToolsByName: (names: string[]) => {
        activeTools.length = 0;
        activeTools.push(...names);
      },
      prompt: async () => {
        prompted += 1;
      },
    };
    const events: JsonObject[] = [];
    const controller = new PlanController(
      new PlanStore(),
      {} as ModelRuntime,
      fakeRuntime(session),
      (event) => events.push(event),
    );
    controller.activateSession(manager.getBranch());
    controller.submit(content, manager.getSessionId());
    const result = await controller.execute("current");
    assert.equal(result.context, "current");
    assert.equal(prompted, 1);
    assert.equal(controller.current(), false);
    assert.ok(events.some((event) => event.type === "plan_mode_state"));
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("PLAN_MODE_POLICY denies mutations but keeps reads unclassified", () => {
  const execIntent = {
    tool: "bash",
    normalizedInput: { command: "touch x" },
    atoms: [{
      kind: "exec",
      executable: "touch",
      argv: ["x"],
      cwd: "/workspace",
      environment: {},
    }],
  } as PermissionIntent;
  assert.equal(
    evaluatePermission(execIntent, PLAN_MODE_POLICY.permissionRules).effect,
    "deny",
  );
  const readIntent = {
    tool: "read",
    normalizedInput: { path: "a.ts" },
    atoms: [{
      kind: "file",
      operation: "read",
      path: "/workspace/a.ts",
    }],
  } as PermissionIntent;
  assert.equal(
    evaluatePermission(readIntent, PLAN_MODE_POLICY.permissionRules).effect,
    "ask",
  );
});
