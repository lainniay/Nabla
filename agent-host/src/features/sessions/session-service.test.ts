import assert from "node:assert/strict";
import test from "node:test";

import type { RuntimeSupervisor } from "../../runtime/runtime-supervisor.ts";
import type { PlanModePort } from "../plans/plan-controller.ts";
import { SessionService } from "./session-service.ts";

const planMode: PlanModePort = {
  current: () => false,
  set: () => ({ active: false, activeTools: [] }),
};

function stubSupervisor(
  overrides: Partial<RuntimeSupervisor> = {},
): RuntimeSupervisor {
  return {
    requireIdle: () =>
      ({
        session: { isIdle: true },
      }) as never,
    current: () =>
      ({
        session: {
          clearQueue: () => ({
            steering: ["s1"],
            followUp: ["f1"],
          }),
        },
      }) as never,
    newSession: async () => ({ cancelled: false }),
    switchSession: async () => ({ cancelled: false }),
    ...overrides,
  } as unknown as RuntimeSupervisor;
}

test("newSession rejects busy sessions", async () => {
  const service = new SessionService(
    stubSupervisor({
      requireIdle: () => {
        throw new Error("Cannot create a session while the agent is running");
      },
    }),
    planMode,
    () => {},
    () => ({ ok: true }),
  );
  await assert.rejects(service.newSession(), /Cannot create a session/u);
});

test("newSession succeeds and invalidates browser catalogs", async () => {
  let transitions = 0;
  const service = new SessionService(
    stubSupervisor(),
    planMode,
    () => {
      transitions += 1;
    },
    () => ({ activation: true }),
  );
  const result = await service.newSession();
  assert.equal(result.cancelled, false);
  assert.deepEqual(result.activation, { activation: true });
  assert.equal(transitions, 1);
});

test("resume failure propagates and does not invalidate", async () => {
  let transitions = 0;
  const service = new SessionService(
    stubSupervisor({
      switchSession: async () => {
        throw new Error("resume failed");
      },
    }),
    planMode,
    () => {
      transitions += 1;
    },
    () => ({ ok: true }),
  );
  await assert.rejects(
    service.resumeSession({ sessionPath: "/missing" }),
    /resume failed/u,
  );
  assert.equal(transitions, 0);
});

test("clearQueue joins steering and follow-up text", () => {
  const service = new SessionService(
    stubSupervisor(),
    planMode,
    () => {},
    () => ({ ok: true }),
  );
  assert.deepEqual(service.clearQueue(), {
    steering: ["s1"],
    followUp: ["f1"],
    restoredText: "s1\n\nf1",
  });
});
