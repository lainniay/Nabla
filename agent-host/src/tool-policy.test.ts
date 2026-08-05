import assert from "node:assert/strict";
import test from "node:test";

import {
  isHighRiskCommand,
  isManagedWorktreeCommand,
  stripRedundantWorkspaceCd,
} from "./policy/tool-policy.ts";

test("high-risk command detection is advisory and identifies UI warnings", () => {
  assert.equal(isHighRiskCommand("rm -rf target"), true);
  assert.equal(isHighRiskCommand("cargo test"), false);
});

test("managed worktree detection covers common Git wrappers", () => {
  assert.equal(isManagedWorktreeCommand("git worktree add ../branch"), true);
  assert.equal(isManagedWorktreeCommand("env git -C . worktree remove ../branch"), true);
  assert.equal(isManagedWorktreeCommand("git worktree list"), false);
});

test("removes only a redundant cd to the current workspace", () => {
  const cwd = process.cwd();
  assert.equal(
    stripRedundantWorkspaceCd(`cd ${cwd} && cargo check`, cwd),
    "cargo check",
  );
  assert.equal(
    stripRedundantWorkspaceCd("cd .. && cargo check", cwd),
    "cd .. && cargo check",
  );
});
