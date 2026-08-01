import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { parseBootstrapState } from "./protocol/contracts.ts";

test("shared bootstrap fixture satisfies the TypeScript protocol contract", () => {
  const fixturePath = new URL(
    "../../protocol-fixtures/bootstrap-state.json",
    import.meta.url,
  );
  const parsed = parseBootstrapState(
    JSON.parse(readFileSync(fixturePath, "utf8")) as unknown,
  );

  assert.equal(parsed.scopeId, "session-contract");
  assert.equal(parsed.goal.goal?.tasks[0]?.description, "Verify the contract");
  assert.equal(parsed.agents.pending[0]?.id, "agent-pending");
  assert.equal(parsed.pendingIntegrations[0]?.integration.patchBytes, 512);
  assert.equal(parsed.context.pruning.length, 3);
});
