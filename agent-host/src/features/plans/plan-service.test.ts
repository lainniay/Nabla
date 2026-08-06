import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import {
  ModelRuntime,
  SessionManager,
} from "@earendil-works/pi-coding-agent";

import {
  PLAN_ENTRY_TYPE,
  PlanStore,
  type PlanArtifact,
  type PlanContent,
} from "../../plan.ts";
import type { RuntimeAccess } from "../../runtime/runtime-access.ts";
import { PlanModeService } from "../../runtime/plan-mode-service.ts";
import type { JsonObject } from "../../protocol/validation.ts";
import { PlanService } from "./plan-service.ts";

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
  prompt: () => Promise<void>;
}): RuntimeAccess {
  return {
    current: () => ({ session }) as never,
    requireIdle: () => ({ session }) as never,
    sessionGeneration: () => 1,
  };
}

test("submit bumps revision and snapshot exposes the artifact", async () => {
  const root = mkdtempSync(join(tmpdir(), "nabla-plan-service-"));
  try {
    const manager = SessionManager.create(root, root);
    const session = {
      isIdle: true,
      sessionId: manager.getSessionId(),
      sessionManager: manager,
      prompt: async () => undefined,
    };
    const service = new PlanService(
      new PlanStore(),
      {} as ModelRuntime,
      fakeRuntime(session),
      new PlanModeService(),
      () => {},
    );
    const first = service.submit(content, manager.getSessionId());
    const second = service.submit(content, manager.getSessionId());
    assert.equal(first.revision, 1);
    assert.equal(second.revision, 2);
    assert.equal(service.snapshot()?.id, second.id);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("restore reads the plan entry from a session branch", async () => {
  const root = mkdtempSync(join(tmpdir(), "nabla-plan-service-"));
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
    const service = new PlanService(
      new PlanStore(),
      {} as ModelRuntime,
      fakeRuntime({
        isIdle: true,
        sessionId: manager.getSessionId(),
        sessionManager: manager,
        prompt: async () => undefined,
      }),
      new PlanModeService(),
      () => {},
    );
    const events: JsonObject[] = [];
    const publishing = new PlanService(
      new PlanStore(),
      {} as ModelRuntime,
      fakeRuntime({
        isIdle: true,
        sessionId: manager.getSessionId(),
        sessionManager: manager,
        prompt: async () => undefined,
      }),
      new PlanModeService(),
      (event) => events.push(event),
    );
    assert.equal(service.restore(manager.getBranch())?.id, "plan-1");
    publishing.onSessionActivated(manager.getBranch());
    assert.deepEqual(events[0]?.artifact, artifact);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("current execute starts a normal turn and exits plan mode", async () => {
  const root = mkdtempSync(join(tmpdir(), "nabla-plan-service-"));
  try {
    const manager = SessionManager.create(root, root);
    let prompted = 0;
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
      prompt: async () => {
        prompted += 1;
      },
    };
    const planMode = new PlanModeService();
    planMode.restore(session as never, true);
    const events: JsonObject[] = [];
    const service = new PlanService(
      new PlanStore(),
      {} as ModelRuntime,
      fakeRuntime(session as never),
      planMode,
      (event) => events.push(event),
    );
    service.submit(content, manager.getSessionId());
    const result = await service.execute("current");
    assert.equal(result.context, "current");
    assert.equal(prompted, 1);
    assert.equal(planMode.current(), false);
    assert.ok(events.some((event) => event.type === "plan_mode_state"));
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
