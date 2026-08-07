import assert from "node:assert/strict";
import {
  mkdirSync,
  mkdtempSync,
  realpathSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import {
  canonicalizePath,
  normalizePath,
  normalizeToolInputPaths,
  workspacePathError,
} from "./path.ts";

test("workspace path guard allows workspace files and rejects escapes", async () => {
  const root = mkdtempSync(join(tmpdir(), "nabla-path-"));
  try {
    const workspace = join(root, "workspace");
    mkdirSync(workspace);
    writeFileSync(join(workspace, "existing.txt"), "x");
    symlinkSync(root, join(workspace, "escape"));
    writeFileSync(join(root, "secret.txt"), "x");
    assert.equal(
      await workspacePathError(workspace, "existing.txt"),
      undefined,
    );
    assert.equal(
      await workspacePathError(workspace, "nested/new.txt"),
      undefined,
    );
    assert.match(
      (await workspacePathError(workspace, "../outside.txt")) ?? "",
      /outside the workspace/u,
    );
    assert.match(
      (await workspacePathError(workspace, "escape/secret.txt")) ?? "",
      /outside the workspace/u,
    );
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("normalizePath rewrites workspace paths to relative", () => {
  assert.equal(normalizePath("/workspace/src/lib.rs", "/workspace"), "src/lib.rs");
  assert.equal(normalizePath("/workspace", "/workspace"), ".");
  assert.equal(normalizePath("/etc/passwd", "/workspace"), undefined);
  assert.equal(normalizePath("src/lib.rs", "/workspace"), undefined);
});

test("normalizeToolInputPaths handles path and destination", () => {
  const input: Record<string, unknown> = {
    path: "/workspace/a.ts",
    destination: "/workspace/b.ts",
    other: "/workspace/c.ts",
  };
  normalizeToolInputPaths(input, "/workspace");
  assert.deepEqual(input, {
    path: "a.ts",
    destination: "b.ts",
    other: "/workspace/c.ts",
  });
});

test("canonicalizePath resolves the nearest existing ancestor", () => {
  const root = mkdtempSync(join(tmpdir(), "nabla-canonical-"));
  try {
    mkdirSync(join(root, "existing"));
    const result = canonicalizePath(root, "existing/new/deep/file.ts");
    assert.equal(
      result,
      join(realpathSync(join(root, "existing")), "new", "deep", "file.ts"),
    );
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
