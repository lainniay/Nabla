import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { parseBootstrapState } from "./protocol/contracts.ts";
import { parseFileReferenceEnvelope } from "./protocol/message-content.ts";
import type { SessionHistoryItem } from "./session-navigation.ts";
import type { WorkspaceGrantSnapshot } from "./permissions/approvals/workspace-store.ts";

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

test("shared turn-boundary fixture preserves exact and estimated history variants", () => {
  const fixture = JSON.parse(
    readFileSync(
      new URL(
        "../../protocol-fixtures/session-history-turn-boundary.json",
        import.meta.url,
      ),
      "utf8",
    ),
  ) as SessionHistoryItem[];
  assert.equal(fixture.length, 2);
  assert.deepEqual(
    fixture.map((item) =>
      item.kind === "turnBoundary"
        ? [item.turnId, item.durationMs, item.estimated]
        : null,
    ),
    [
      ["turn-exact", 65_000, false],
      ["legacy-entry-1", 12_000, true],
    ],
  );
});

test("shared file-reference fixture parses the versioned wire envelope", () => {
  const fixture = JSON.parse(
    readFileSync(
      new URL(
        "../../protocol-fixtures/nabla.file-references.v1.json",
        import.meta.url,
      ),
      "utf8",
    ),
  ) as { wire: string; message: string };
  const parsed = parseFileReferenceEnvelope(fixture.wire);
  assert.equal(parsed?.message, fixture.message);
  assert.equal(parsed?.references[0]?.path, "src/lib.rs");
  assert.equal(parsed?.references[0]?.mode, "snapshot");
});

test("shared persistent-approval fixture uses the cross-language wire shape", () => {
  const fixture = JSON.parse(
    readFileSync(
      new URL(
        "../../protocol-fixtures/nabla.workspace-grants.v3.json",
        import.meta.url,
      ),
      "utf8",
    ),
  ) as WorkspaceGrantSnapshot;
  assert.equal(fixture.workspace, "/workspace");
  assert.equal(fixture.grants[0]?.scope, "workspace");
  assert.equal(fixture.grants[0]?.matchers[0]?.kind, "exec");
});
