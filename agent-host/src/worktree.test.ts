import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { WorktreeManager } from "./worktree.ts";

function git(cwd: string, ...args: string[]): string {
  return execFileSync("git", ["-C", cwd, ...args], {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
}

test("software-managed worktree snapshots an unborn dirty repository and applies only agent changes", async () => {
  const root = await mkdtemp(join(tmpdir(), "nabla-worktree-unborn-"));
  const repo = join(root, "repo");
  await mkdir(repo);
  git(repo, "init");
  await writeFile(
    join(repo, ".gitignore"),
    "ignored.txt\n.env\nnode_modules/\n",
  );
  await writeFile(join(repo, "source.txt"), "original\n");
  await writeFile(join(repo, ".env"), "SECRET=hidden\n");
  await writeFile(join(repo, "ignored.txt"), "build output\n");
  await mkdir(join(repo, "node_modules"));
  await writeFile(join(repo, "node_modules", "dependency.js"), "module\n");
  const statusBefore = git(repo, "status", "--porcelain=v1");
  const manager = new WorktreeManager({
    rootDir: join(root, "managed"),
    credentialPath: (path) => path.endsWith("/.env"),
  });

  try {
    const prepared = await manager.prepare(
      "agent-1",
      repo,
      { mode: "auto", integration: "source" },
    );
    assert.equal(prepared.backend, "worktree");
    assert.ok(prepared.record);
    assert.match(prepared.warning ?? "", /node_modules/u);
    assert.equal(
      await readFile(join(prepared.executionCwd, "source.txt"), "utf8"),
      "original\n",
    );
    await assert.rejects(readFile(join(prepared.executionCwd, ".env"), "utf8"));
    assert.equal(git(repo, "status", "--porcelain=v1"), statusBefore);
    assert.throws(() => git(repo, "rev-parse", "--verify", "HEAD"));

    await writeFile(
      join(prepared.executionCwd, "source.txt"),
      "agent update\n",
    );
    await writeFile(join(prepared.executionCwd, "new.txt"), "new file\n");
    const captured = await manager.capture(prepared.record);
    assert.equal(captured.hasChanges, true);
    assert.deepEqual(captured.record.changedPaths.sort(), [
      "new.txt",
      "source.txt",
    ]);
    await manager.annotate(captured.record, {
      profile: "worker",
      task: "update source",
      direct: true,
      planReadOnly: false,
      model: "test/model",
      originSessionId: "session-1",
      result: { status: "completed", summary: "updated" },
    });
    const recovery = await new WorktreeManager({
      rootDir: join(root, "managed"),
    }).listRecoverable(repo);
    const recovered = recovery.records;
    assert.deepEqual(recovery.warnings, []);
    assert.equal(recovered.length, 1);
    assert.equal(recovered[0]?.recovery?.task, "update source");
    const integrated = await manager.integrate(captured.record);
    assert.equal(integrated.status, "applied");
    assert.equal(await readFile(join(repo, "source.txt"), "utf8"), "agent update\n");
    assert.equal(await readFile(join(repo, "new.txt"), "utf8"), "new file\n");
    assert.throws(() => git(repo, "rev-parse", "--verify", "HEAD"));
    const removed = await manager.pruneTerminalArtifacts(
      repo,
      Date.now() + 1_000,
      0,
    );
    assert.equal(removed, 1);
    await assert.rejects(
      readFile(join(captured.record.artifactDirectory, "record.json"), "utf8"),
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("snapshot preserves the user's staged index while including the final dirty working tree", async () => {
  const root = await mkdtemp(join(tmpdir(), "nabla-worktree-index-"));
  const repo = join(root, "repo");
  await mkdir(repo);
  git(repo, "init");
  git(repo, "config", "user.name", "Test");
  git(repo, "config", "user.email", "test@example.com");
  await writeFile(join(repo, "file.txt"), "committed\n");
  git(repo, "add", "file.txt");
  git(repo, "commit", "-m", "initial");
  await writeFile(join(repo, "file.txt"), "staged\n");
  git(repo, "add", "file.txt");
  await writeFile(join(repo, "file.txt"), "working\n");
  await writeFile(join(repo, "untracked.txt"), "untracked\n");
  const statusBefore = git(repo, "status", "--porcelain=v1");
  const stagedBefore = git(repo, "diff", "--cached", "--binary");
  const manager = new WorktreeManager({ rootDir: join(root, "managed") });
  try {
    const prepared = await manager.prepare(
      "agent-1",
      repo,
      { mode: "worktree", integration: "ask" },
    );
    assert.equal(
      await readFile(join(prepared.executionCwd, "file.txt"), "utf8"),
      "working\n",
    );
    assert.equal(
      await readFile(join(prepared.executionCwd, "untracked.txt"), "utf8"),
      "untracked\n",
    );
    assert.equal(git(repo, "status", "--porcelain=v1"), statusBefore);
    assert.equal(git(repo, "diff", "--cached", "--binary"), stagedBefore);
    await manager.discard(prepared.record!);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("serialized integration accepts disjoint worker patches", async () => {
  const root = await mkdtemp(join(tmpdir(), "nabla-worktree-parallel-"));
  const repo = join(root, "repo");
  await mkdir(repo);
  git(repo, "init");
  git(repo, "config", "user.name", "Test");
  git(repo, "config", "user.email", "test@example.com");
  await writeFile(join(repo, "a.txt"), "a\n");
  await writeFile(join(repo, "b.txt"), "b\n");
  git(repo, "add", "a.txt", "b.txt");
  git(repo, "commit", "-m", "initial");
  const manager = new WorktreeManager({ rootDir: join(root, "managed") });
  try {
    const first = await manager.prepare(
      "agent-1",
      repo,
      { mode: "worktree", integration: "auto" },
    );
    const second = await manager.prepare(
      "agent-2",
      repo,
      { mode: "worktree", integration: "auto" },
    );
    await writeFile(join(first.executionCwd, "a.txt"), "agent one\n");
    await writeFile(join(second.executionCwd, "b.txt"), "agent two\n");
    const [firstPatch, secondPatch] = await Promise.all([
      manager.capture(first.record!),
      manager.capture(second.record!),
    ]);
    const [firstResult, secondResult] = await Promise.all([
      manager.integrate(firstPatch.record),
      manager.integrate(secondPatch.record),
    ]);
    assert.equal(firstResult.status, "applied");
    assert.equal(secondResult.status, "applied");
    assert.equal(await readFile(join(repo, "a.txt"), "utf8"), "agent one\n");
    assert.equal(await readFile(join(repo, "b.txt"), "utf8"), "agent two\n");
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("capture preserves leading whitespace in Git paths", async () => {
  const root = await mkdtemp(join(tmpdir(), "nabla-worktree-path-"));
  const repo = join(root, "repo");
  await mkdir(repo);
  git(repo, "init");
  git(repo, "config", "user.name", "Test");
  git(repo, "config", "user.email", "test@example.com");
  await writeFile(join(repo, "base.txt"), "base\n");
  git(repo, "add", "base.txt");
  git(repo, "commit", "-m", "initial");
  const manager = new WorktreeManager({ rootDir: join(root, "managed") });
  try {
    const prepared = await manager.prepare(
      "agent-path",
      repo,
      { mode: "worktree", integration: "ask" },
    );
    await writeFile(join(prepared.executionCwd, " leading.txt"), "kept\n");
    const captured = await manager.capture(prepared.record!);
    assert.deepEqual(captured.record.changedPaths, [" leading.txt"]);
    const integrated = await manager.integrate(captured.record);
    assert.equal(integrated.status, "applied");
    assert.equal(await readFile(join(repo, " leading.txt"), "utf8"), "kept\n");
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("integration is idempotent across manager instances", async () => {
  const root = await mkdtemp(join(tmpdir(), "nabla-worktree-idempotent-"));
  const repo = join(root, "repo");
  const managed = join(root, "managed");
  await mkdir(repo);
  git(repo, "init");
  git(repo, "config", "user.name", "Test");
  git(repo, "config", "user.email", "test@example.com");
  await writeFile(join(repo, "file.txt"), "base\n");
  git(repo, "add", "file.txt");
  git(repo, "commit", "-m", "initial");
  const firstManager = new WorktreeManager({ rootDir: managed });
  const secondManager = new WorktreeManager({ rootDir: managed });
  try {
    const prepared = await firstManager.prepare(
      "agent-idempotent",
      repo,
      { mode: "worktree", integration: "auto" },
    );
    await writeFile(join(prepared.executionCwd, "file.txt"), "updated\n");
    const captured = await firstManager.capture(prepared.record!);
    const [first, second] = await Promise.all([
      firstManager.integrate(captured.record),
      secondManager.integrate(captured.record),
    ]);
    assert.deepEqual([first.status, second.status], ["applied", "applied"]);
    assert.equal(await readFile(join(repo, "file.txt"), "utf8"), "updated\n");
    const third = await secondManager.integrate(captured.record);
    assert.equal(third.status, "applied");
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("integration reconciles a crash after patch application", async () => {
  const root = await mkdtemp(join(tmpdir(), "nabla-worktree-reconcile-"));
  const repo = join(root, "repo");
  const managed = join(root, "managed");
  await mkdir(repo);
  git(repo, "init");
  git(repo, "config", "user.name", "Test");
  git(repo, "config", "user.email", "test@example.com");
  await writeFile(join(repo, "file.txt"), "base\n");
  git(repo, "add", "file.txt");
  git(repo, "commit", "-m", "initial");
  const manager = new WorktreeManager({ rootDir: managed });
  try {
    const prepared = await manager.prepare(
      "agent-crash",
      repo,
      { mode: "worktree", integration: "auto" },
    );
    await writeFile(join(prepared.executionCwd, "file.txt"), "updated\n");
    const captured = await manager.capture(prepared.record!);
    const recordPath = join(captured.record.artifactDirectory, "record.json");
    const persisted = JSON.parse(await readFile(recordPath, "utf8"));
    persisted.integrationStatus = "applying";
    persisted.applyStartedAt = new Date().toISOString();
    await writeFile(recordPath, `${JSON.stringify(persisted, null, 2)}\n`);
    git(repo, "apply", "--binary", captured.record.patchPath);

    const recovered = await new WorktreeManager({ rootDir: managed }).integrate(
      captured.record,
    );
    assert.equal(recovered.status, "applied");
    assert.equal(recovered.record.integrationStatus, "applied");
    assert.equal(await readFile(join(repo, "file.txt"), "utf8"), "updated\n");
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("auto serializes by falling back outside Git while explicit worktree fails", async () => {
  const root = await mkdtemp(join(tmpdir(), "nabla-worktree-nongit-"));
  const manager = new WorktreeManager({ rootDir: join(root, "managed") });
  try {
    const automatic = await manager.prepare(
      "agent-1",
      root,
      { mode: "auto", integration: "source" },
    );
    assert.equal(automatic.backend, "shared_fallback");
    assert.match(automatic.warning ?? "", /serialized/u);
    await assert.rejects(
      manager.prepare(
        "agent-2",
        root,
        { mode: "worktree", integration: "source" },
      ),
      /not a Git repository/u,
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("recovery scan reports a corrupt record without hiding other state", async () => {
  const root = await mkdtemp(join(tmpdir(), "nabla-worktree-corrupt-"));
  const workspace = join(root, "workspace");
  const managed = join(root, "managed");
  const artifact = join(managed, "workspace-hash", "agent-corrupt");
  await mkdir(workspace);
  await mkdir(artifact, { recursive: true });
  await writeFile(join(artifact, "record.json"), "{not-json");

  try {
    const scan = await new WorktreeManager({ rootDir: managed }).listRecoverable(
      workspace,
    );
    assert.deepEqual(scan.records, []);
    assert.equal(scan.warnings.length, 1);
    assert.match(scan.warnings[0] ?? "", /record\.json/u);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("integration is all-or-nothing and preserves a conflicting worktree", async () => {
  const root = await mkdtemp(join(tmpdir(), "nabla-worktree-conflict-"));
  const repo = join(root, "repo");
  await mkdir(repo);
  git(repo, "init");
  git(repo, "config", "user.name", "Test");
  git(repo, "config", "user.email", "test@example.com");
  await writeFile(join(repo, "file.txt"), "base\n");
  git(repo, "add", "file.txt");
  git(repo, "commit", "-m", "initial");
  const manager = new WorktreeManager({ rootDir: join(root, "managed") });
  try {
    const prepared = await manager.prepare(
      "agent-1",
      repo,
      { mode: "worktree", integration: "ask" },
    );
    assert.ok(prepared.record);
    await writeFile(join(prepared.executionCwd, "file.txt"), "agent\n");
    const captured = await manager.capture(prepared.record);
    await writeFile(join(repo, "file.txt"), "user\n");
    const integrated = await manager.integrate(captured.record);
    assert.equal(integrated.status, "conflicted");
    assert.equal(await readFile(join(repo, "file.txt"), "utf8"), "user\n");
    assert.equal(
      await readFile(join(prepared.executionCwd, "file.txt"), "utf8"),
      "agent\n",
    );
    const resolution = await manager.prepareResolution(
      "agent-1-resolver",
      captured.record,
    );
    assert.equal(captured.record.resolutionAttempts, 1);
    await assert.rejects(
      manager.prepareResolution("agent-1-resolver-2", captured.record),
      /already been used/u,
    );
    assert.deepEqual(resolution.conflictPaths, ["file.txt"]);
    await writeFile(
      join(resolution.isolation.executionCwd, "file.txt"),
      "user + agent\n",
    );
    const resolvedPatch = await manager.capture(resolution.isolation.record);
    await manager.assertResolved(resolvedPatch.record);
    const appliedResolution = await manager.integrate(resolvedPatch.record);
    assert.equal(appliedResolution.status, "applied");
    await manager.resolvedBy(captured.record, "agent-1-resolver");
    assert.equal(
      await readFile(join(repo, "file.txt"), "utf8"),
      "user + agent\n",
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
