import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { ShellAdapter } from "../../permissions/adapters/shell.ts";
import type { ToolContext } from "../../permissions/model.ts";
import { agentToolResource, permissionIntentForTool } from "./tool-intent.ts";

const context: ToolContext = {
  requestId: "request-1",
  toolCallId: "tool-1",
  sessionId: "session-1",
  workspaceId: "workspace-1",
  cwd: "/workspace",
};

test("bash commands map to unchanged exec atoms", () => {
  const intent = permissionIntentForTool(
    context,
    "bash",
    { command: "echo hi" },
    new ShellAdapter(),
  );
  assert.equal(intent.tool, "bash");
  assert.deepEqual(intent.atoms, [{
    kind: "exec",
    executable: "echo",
    argv: ["hi"],
    cwd: "/workspace",
    environment: {},
  }]);
});

test("file tools map to the specialized file adapter", () => {
  const intent = permissionIntentForTool(
    context,
    "edit",
    { path: "src/a.ts" },
    new ShellAdapter(),
  );
  assert.equal(intent.tool, "edit");
  assert.deepEqual(
    intent.atoms.map((atom) =>
      atom.kind === "file" ? [atom.operation, atom.path] : null,
    ),
    [["write", "/workspace/src/a.ts"]],
  );
});

test("write distinguishes existing files from new files", () => {
  const root = mkdtempSync(join(tmpdir(), "nabla-tool-intent-"));
  try {
    writeFileSync(join(root, "existing.txt"), "x");
    const existing = permissionIntentForTool(
      { ...context, cwd: root },
      "write",
      { path: "existing.txt" },
      new ShellAdapter(),
    );
    assert.deepEqual(
      existing.atoms.map((atom) =>
        atom.kind === "file" ? atom.operation : null,
      ),
      ["truncate", "write"],
    );
    const created = permissionIntentForTool(
      { ...context, cwd: root },
      "write",
      { path: "missing.txt" },
      new ShellAdapter(),
    );
    assert.deepEqual(
      created.atoms.map((atom) =>
        atom.kind === "file" ? atom.operation : null,
      ),
      ["create", "write"],
    );
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("mcp, delegate_task, and unknown tools keep their opaque fallbacks", () => {
  const mcp = permissionIntentForTool(
    context,
    "mcp__server__method",
    { x: 1 },
    new ShellAdapter(),
  );
  if (mcp.atoms[0]?.kind !== "opaque_code") {
    throw new Error("mcp intent must be opaque");
  }
  assert.equal(mcp.atoms[0].runtime, "mcp:server/method");
  assert.equal(
    mcp.atoms[0].reason,
    "MCP method effects are declared by the server, not the host",
  );
  assert.ok(mcp.atoms[0].digest.length > 0);

  const delegate = permissionIntentForTool(
    context,
    "delegate_task",
    { task: "work", profile: "worker" },
    new ShellAdapter(),
  );
  if (delegate.atoms[0]?.kind !== "opaque_code") {
    throw new Error("delegate intent must be opaque");
  }
  assert.equal(delegate.atoms[0].runtime, "agent:spawn");
  assert.equal(delegate.atoms[0].reason, "delegated agent action");
  assert.ok(delegate.atoms[0].digest.length > 0);

  const unknown = permissionIntentForTool(
    context,
    "custom_tool",
    { value: 1 },
    new ShellAdapter(),
  );
  if (unknown.atoms[0]?.kind !== "opaque_code") {
    throw new Error("unknown tool intent must be opaque");
  }
  assert.equal(unknown.atoms[0].runtime, "tool:custom_tool");
});

test("agentToolResource normalizes commands and workspace paths", () => {
  assert.equal(agentToolResource("/w", "a/b.ts", undefined), "a/b.ts");
  assert.equal(agentToolResource("/w", undefined, "  git   status  "), "git status");
  assert.equal(agentToolResource("/w", undefined, undefined), "*");
});
