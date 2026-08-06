import assert from "node:assert/strict";
import test from "node:test";

import { BootstrapService } from "./bootstrap-service.ts";

test("bootstrap aggregates the read-only snapshot shape", () => {
  const service = new BootstrapService();
  const snapshot = service.snapshot({
    scopeId: "session-1",
    planMode: { active: true, activeTools: ["read", "ask_user"] },
    artifact: null,
    resources: { revision: 1 } as never,
    agents: { revision: 1 } as never,
    context: { revision: 1 } as never,
    pendingIntegrations: [],
    warnings: ["w"],
  });
  assert.equal(snapshot.scopeId, "session-1");
  assert.equal(snapshot.planMode.active, true);
  assert.equal(snapshot.plan.artifact, null);
  assert.deepEqual(snapshot.warnings, ["w"]);
  assert.deepEqual(snapshot.pendingIntegrations, []);
});
