import assert from "node:assert/strict";
import { mkdtemp, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { workspacePathError } from "./workspace.ts";

test("workspace path guard allows workspace files and rejects escapes", async () => {
  const workspace = await mkdtemp(join(tmpdir(), "nabla-workspace-"));
  const outside = await mkdtemp(join(tmpdir(), "nabla-outside-"));
  try {
    await writeFile(join(workspace, "existing.txt"), "ok");
    await writeFile(join(outside, "secret.txt"), "secret");
    await symlink(outside, join(workspace, "escape"));

    assert.equal(await workspacePathError(workspace, "existing.txt"), undefined);
    assert.equal(await workspacePathError(workspace, "nested/new.txt"), undefined);
    assert.match(
      (await workspacePathError(workspace, "../outside.txt")) ?? "",
      /outside the workspace/u,
    );
    assert.match(
      (await workspacePathError(workspace, "escape/secret.txt")) ?? "",
      /follow a path outside/u,
    );
  } finally {
    await rm(workspace, { recursive: true, force: true });
    await rm(outside, { recursive: true, force: true });
  }
});
