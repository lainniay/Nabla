import assert from "node:assert/strict";
import test from "node:test";

import type { AgentSession } from "@earendil-works/pi-coding-agent";

import {
  PLAN_TOOLS,
  PlanModeService,
  STANDARD_TOOLS,
} from "./plan-mode-service.ts";

function fakeSession(overrides: Partial<AgentSession> = {}): AgentSession {
  const state: { active: string[] } = { active: [] };
  return {
    isIdle: true,
    getActiveToolNames: () => state.active,
    setActiveToolsByName: (names: string[]) => {
      state.active = names;
    },
    ...overrides,
  } as unknown as AgentSession;
}

test("restore applies the expected tools and tracks active state", () => {
  const service = new PlanModeService();
  const session = fakeSession();
  assert.deepEqual(service.restore(session, true), [...PLAN_TOOLS]);
  assert.equal(service.current(), true);
  assert.deepEqual(service.restore(session, false), [...STANDARD_TOOLS]);
  assert.equal(service.current(), false);
});

test("set rejects busy sessions without changing mode", () => {
  const service = new PlanModeService();
  const session = fakeSession({ isIdle: false });
  assert.throws(
    () => service.set(session, true),
    /Cannot switch mode while the agent is running/u,
  );
  assert.equal(service.current(), false);
});

test("missing tools roll back to the previous tool set", () => {
  const service = new PlanModeService();
  const attempts: string[][] = [];
  const session = fakeSession();
  session.setActiveToolsByName = (names: string[]) => {
    attempts.push(names);
  };
  assert.throws(
    () => service.restore(session, true),
    /Pi did not register required tools/u,
  );
  assert.deepEqual(attempts, [
    [...PLAN_TOOLS],
    [],
  ]);
  assert.equal(service.current(), false);
});
