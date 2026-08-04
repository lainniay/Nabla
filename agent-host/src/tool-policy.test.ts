import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import {
  isManagedWorktreeCommand,
  isSafeReadOnlyCommand,
  isSafeReadOnlyWorkspaceCommand,
  stripRedundantWorkspaceCd,
  toolCallCanMutate,
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

test("recognizes simple read-only compound commands inside the workspace", () => {
  const workspace = mkdtempSync(join(tmpdir(), "nabla-tool-policy-"));
  try {
    mkdirSync(join(workspace, "agent-host", "src"), { recursive: true });
    writeFileSync(join(workspace, "agent-host", "src", "main.ts"), "export {};\n");
    const command = [
      `cd ${workspace}`,
      "head -60 agent-host/src/main.ts",
      'echo "===HARNESS==="',
      "sed -n '1,20p' agent-host/src/main.ts",
      "ls agent-host/src agent-host/src/policy",
      "wc -l agent-host/src/*.ts 2>/dev/null",
      `cat ${workspace}/agent-host/src/main.ts 2>/dev/null || echo "missing"`,
      `git -C ${workspace} remote -v 2>/dev/null`,
      `git -C ${workspace} status --short`,
      `git -C ${workspace} log -5 --oneline`,
    ].join(" && ");
    assert.equal(isSafeReadOnlyWorkspaceCommand(command, workspace), true);
  } finally {
    rmSync(workspace, { recursive: true, force: true });
  }
});

test("rejects workspace escapes, substitutions, pipes, and mutating segments", () => {
  const workspace = mkdtempSync(join(tmpdir(), "nabla-tool-policy-"));
  try {
    writeFileSync(join(workspace, "file.txt"), "value\n");
    for (const command of [
      "head /etc/passwd",
      "echo $(cat file.txt)",
      'echo "$(cat file.txt)"',
      "head file.txt | tee copy.txt",
      "cat /etc/passwd || echo missing",
      "cat file.txt || rm -rf target",
      `git -C ${workspace} remote add origin https://example.com/repo.git`,
      `git -C ${workspace} branch -D main`,
      "git -C .. status --short",
      "wc -l file.txt > counts.txt",
      "wc -l file.txt 2> errors.txt",
      "wc -l {src,/etc}/*",
      "ls ~",
      "ls -L .",
      "head file.txt; rm -rf target",
      "cd .. && head secret.txt",
      "sed -i 's/a/b/' file.txt",
    ]) {
      assert.equal(
        isSafeReadOnlyWorkspaceCommand(command, workspace),
        false,
        command,
      );
    }
  } finally {
    rmSync(workspace, { recursive: true, force: true });
  }
});

test("removes only a redundant cd to the current workspace", () => {
  const workspace = mkdtempSync(join(tmpdir(), "nabla-tool-policy-"));
  try {
    assert.equal(
      stripRedundantWorkspaceCd(
        `cd "${workspace}" && wc -l src/*.rs 2>/dev/null`,
        workspace,
      ),
      "wc -l src/*.rs 2>/dev/null",
    );
    assert.equal(
      stripRedundantWorkspaceCd("cd .. && ls", workspace),
      "cd .. && ls",
    );
  } finally {
    rmSync(workspace, { recursive: true, force: true });
  }
});

test("repository-summary shell reads are non-mutating as a complete invocation", () => {
  const workspace = mkdtempSync(join(tmpdir(), "nabla-tool-policy-"));
  try {
    const readCommand = [
      `git -C ${workspace} remote -v 2>/dev/null`,
      'echo "---"',
      `git -C ${workspace} status --short`,
    ].join("; ");
    assert.equal(toolCallCanMutate("bash", readCommand, workspace), false);
    assert.equal(
      toolCallCanMutate(
        "bash",
        `git -C ${workspace} remote add origin https://example.com/repo.git`,
        workspace,
      ),
      true,
    );
  } finally {
    rmSync(workspace, { recursive: true, force: true });
  }
});
