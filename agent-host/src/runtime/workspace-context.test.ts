import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { buildWorkspaceContext } from "./workspace-context.ts";

test("renders cwd, git root, and a filtered directory tree", () => {
  const root = mkdtempSync(join(tmpdir(), "nabla-workspace-context-"));
  try {
    writeFileSync(join(root, "a.ts"), "x");
    mkdirSync(join(root, "src"));
    writeFileSync(join(root, "src", "b.rs"), "y");
    mkdirSync(join(root, "node_modules"));
    writeFileSync(join(root, "node_modules", "x"), "z");
    writeFileSync(join(root, ".hidden"), "h");

    const context = buildWorkspaceContext(root);
    assert.match(context, new RegExp(`Current working directory: ${root.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&")}`));
    assert.match(context, /Working directory tree:/u);
    assert.match(context, /- a\.ts/u);
    assert.match(context, /- src\//u);
    assert.match(context, /- b\.rs/u);
    assert.doesNotMatch(context, /node_modules/u);
    assert.doesNotMatch(context, /\.hidden/u);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("tree depth and per-directory entry limits apply", () => {
  const root = mkdtempSync(join(tmpdir(), "nabla-workspace-limit-"));
  try {
    mkdirSync(join(root, "a", "b", "c"), { recursive: true });
    for (let index = 0; index < 25; index += 1) {
      writeFileSync(join(root, `f${index}.txt`), "x");
    }

    const context = buildWorkspaceContext(root);
    assert.match(context, /- \.\.\. 6 more entries/u);
    assert.doesNotMatch(context, /- c\//u);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
