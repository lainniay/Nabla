import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";

import type { WorkspaceIdentity } from "../workspace-identity.ts";
import { WorkspaceGrantStore } from "./workspace-store.ts";

test("workspace grants accept wildcard file and shell_command matchers", () => {
  const home = setup();
  try {
    const identity: WorkspaceIdentity = {
      id: "w1",
      canonicalPath: "/ws",
      generationId: "g1",
    };
    writeDocument(home, identity, {
      schemaVersion: 3,
      identity: { canonicalRoot: "/ws", generationId: "g1" },
      grants: [
        {
          scope: "workspace",
          workspaceId: "w1",
          matchers: [{ kind: "file", operation: "*", path: "/ws" }],
        },
        {
          scope: "workspace",
          workspaceId: "w1",
          matchers: [{ kind: "shell_command", pattern: "npm run *" }],
        },
      ],
    });
    const grants = new WorkspaceGrantStore(home).get(identity);
    assert.equal(grants.length, 2);
  } finally {
    rmSync(home, { recursive: true, force: true });
  }
});

test("non-workspace grants are dropped on read", () => {
  const home = setup();
  try {
    const identity: WorkspaceIdentity = {
      id: "w1",
      canonicalPath: "/ws",
      generationId: "g1",
    };
    writeDocument(home, identity, {
      schemaVersion: 3,
      identity: { canonicalRoot: "/ws", generationId: "g1" },
      grants: [
        {
          scope: "session",
          workspaceId: "w1",
          matchers: [{ kind: "file", operation: "read", path: "/ws" }],
        },
      ],
    });
    const grants = new WorkspaceGrantStore(home).get(identity);
    assert.deepEqual(grants, []);
  } finally {
    rmSync(home, { recursive: true, force: true });
  }
});

test("npm_script_digest invalidation keys require path and selector", () => {
  const home = setup();
  try {
    const identity: WorkspaceIdentity = {
      id: "w1",
      canonicalPath: "/ws",
      generationId: "g1",
    };
    writeDocument(home, identity, {
      schemaVersion: 3,
      identity: { canonicalRoot: "/ws", generationId: "g1" },
      grants: [
        {
          scope: "workspace",
          workspaceId: "w1",
          matchers: [{ kind: "file", operation: "read", path: "/ws" }],
          invalidationKeys: [
            { kind: "npm_script_digest", value: "digest" },
          ],
        },
      ],
    });
    const grants = new WorkspaceGrantStore(home).get(identity);
    assert.deepEqual(grants, []);
  } finally {
    rmSync(home, { recursive: true, force: true });
  }
});

function setup(): string {
  return mkdtempSync(join(tmpdir(), "nabla-workspace-store-"));
}

function writeDocument(
  home: string,
  identity: WorkspaceIdentity,
  document: unknown,
): void {
  const path = join(
    home,
    ".nabla",
    "workspaces",
    identity.id,
    "permissions.json",
  );
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, JSON.stringify(document));
}
