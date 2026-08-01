import assert from "node:assert/strict";
import test from "node:test";

import {
  isSafeReadOnlyCommand,
  isManagedWorktreeCommand,
  SAFE_READ_ONLY_COMMAND_PREFIXES,
  THINKING_LEVELS,
} from "./policy/tool-policy.ts";

test("managed worktree detection covers common Git wrappers", () => {
  for (const command of [
    "git worktree add /tmp/worker",
    "git -C /repo worktree remove /tmp/worker",
    "command git worktree prune",
    "env GIT_DIR=/repo/.git git worktree move old new",
    "cd /repo && git worktree repair",
  ]) {
    assert.equal(isManagedWorktreeCommand(command), true, command);
  }
  assert.equal(isManagedWorktreeCommand("git worktree list"), false);
  assert.equal(isManagedWorktreeCommand("git status"), false);
});

test("shared policy exposes one thinking list and read-only Clippy rule", () => {
  assert.deepEqual(THINKING_LEVELS, [
    "off",
    "minimal",
    "low",
    "medium",
    "high",
    "xhigh",
    "max",
  ]);
  assert.ok(SAFE_READ_ONLY_COMMAND_PREFIXES.includes("cargo clippy"));
  assert.equal(isSafeReadOnlyCommand("cargo clippy --all-targets"), true);
  assert.equal(isSafeReadOnlyCommand("cargo test && rm snapshot.txt"), false);
  assert.equal(isSafeReadOnlyCommand("npm test $(touch changed.txt)"), false);
  assert.equal(isSafeReadOnlyCommand("git diff --output=changed.patch"), false);
});
