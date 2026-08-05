import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { ApprovalStore } from "./approval-store.ts";

function fixture() {
  const root = mkdtempSync(join(tmpdir(), "nabla-approvals-"));
  const home = join(root, "home");
  const workspace = join(root, "workspace");
  const otherWorkspace = join(root, "other");
  mkdirSync(home);
  mkdirSync(join(workspace, "src"), { recursive: true });
  mkdirSync(otherWorkspace);
  writeFileSync(join(workspace, "src", "main.ts"), "export {};\n");
  return {
    root,
    home,
    workspace,
    otherWorkspace,
    cleanup: () => rmSync(root, { recursive: true, force: true }),
  };
}

test("persistent approvals survive store recreation and remain project scoped", () => {
  const value = fixture();
  try {
    const first = new ApprovalStore({
      homeDir: value.home,
      createId: () => "rule-1",
      now: () => "2026-08-04T00:00:00.000Z",
    });
    first.allow(value.workspace, "write", { path: "src/main.ts", content: "next" });

    const restored = new ApprovalStore({ homeDir: value.home });
    assert.equal(
      restored.allows(value.workspace, "write", {
        content: "different content is not part of a path rule",
        path: "src/main.ts",
      }),
      true,
    );
    assert.equal(
      restored.allows(value.otherWorkspace, "write", { path: "src/main.ts" }),
      false,
    );
    assert.equal(restored.snapshot(value.workspace).rules[0]?.id, "rule-1");
  } finally {
    value.cleanup();
  }
});

test("directory approvals cover descendants and can be revoked or cleared", () => {
  const value = fixture();
  try {
    const store = new ApprovalStore({
      homeDir: value.home,
      createId: () => "directory-rule",
    });
    store.allow(value.workspace, "write", { path: "src" });
    assert.equal(
      store.allows(value.workspace, "write", { path: "src/new/file.ts" }),
      true,
    );
    assert.equal(store.allows(value.workspace, "write", { path: "README.md" }), false);
    assert.equal(store.revoke(value.workspace, "directory-rule").rules.length, 0);

    store.allow(value.workspace, "write", { path: "src" });
    assert.equal(store.clear(value.workspace).rules.length, 0);
  } finally {
    value.cleanup();
  }
});

test("legacy bash approvals are exact opaque commands and never prefixes", () => {
  const value = fixture();
  try {
    const store = new ApprovalStore({ homeDir: value.home });
    store.allow(value.workspace, "bash", { command: "cargo test -p nabla" });
    assert.equal(
      store.allows(value.workspace, "bash", { command: " cargo  test -p nabla " }),
      true,
    );
    assert.equal(
      store.allows(value.workspace, "bash", { command: "cargo test --all" }),
      false,
    );
    const readOnlyCompound =
      `cd ${value.workspace} && head -20 src/main.ts; echo "done"`;
    store.allow(value.workspace, "bash", { command: readOnlyCompound });
    assert.equal(
      store.allows(value.workspace, "bash", { command: readOnlyCompound }),
      true,
    );
    store.allow(value.workspace, "bash", {
      command: "head src/main.ts; rm -rf target",
    });
    assert.equal(
      store.allows(value.workspace, "bash", {
        command: "head src/main.ts; rm -rf target --extra",
      }),
      false,
    );
  } finally {
    value.cleanup();
  }
});

test("a malformed approvals document fails closed", () => {
  const value = fixture();
  try {
    mkdirSync(join(value.home, ".nabla"));
    writeFileSync(join(value.home, ".nabla", "approvals.json"), "{broken");
    const store = new ApprovalStore({ homeDir: value.home });
    assert.equal(store.snapshot(value.workspace).rules.length, 0);
    assert.equal(
      store.allows(value.workspace, "write", { path: "src/main.ts" }),
      false,
    );
  } finally {
    value.cleanup();
  }
});

test("legacy command_prefix approvals are ignored and require re-approval", () => {
  const value = fixture();
  try {
    mkdirSync(join(value.home, ".nabla"));
    writeFileSync(
      join(value.home, ".nabla", "approvals.json"),
      JSON.stringify({
        schemaVersion: 1,
        rules: [{
          id: "legacy-prefix",
          workspace: value.workspace,
          toolName: "bash",
          kind: "command_prefix",
          value: "cargo test",
          recursive: false,
          summary: "cargo test …",
          createdAt: "2026-01-01T00:00:00.000Z",
        }],
      }),
    );
    const store = new ApprovalStore({ homeDir: value.home });
    assert.equal(store.snapshot(value.workspace).rules.length, 0);
    assert.equal(
      store.allows(value.workspace, "bash", { command: "cargo test --all" }),
      false,
    );
  } finally {
    value.cleanup();
  }
});
