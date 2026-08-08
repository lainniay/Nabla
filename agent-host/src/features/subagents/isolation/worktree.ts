import { randomUUID } from "node:crypto";
import { mkdir, readFile, realpath, rm, stat, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join, relative, resolve, sep } from "node:path";

import { writeAtomicFile, writeAtomicJson } from "../../../persistence/atomic-json.ts";
import {
  assertWorkspaceRelativePath,
  isPathWithin,
} from "../../permissions/filesystem/path.ts";
import { sha256Hex } from "../../permissions/shell/digest.ts";
import { errorMessage } from "../../../protocol/validation.ts";
import { WorktreeArtifactStore } from "./artifact-store.ts";
import { GitClient, type GitResult } from "./git.ts";
import { listRecoverable, pruneTerminalArtifacts } from "./recovery.ts";
import { DEFAULT_LOCK_TIMEOUT_MS, INTERNAL_GIT_IDENTITY } from "./model.ts";
import type {
  AgentIsolationPolicy,
  CapturedWorktree,
  IntegrationResult,
  PreparedIsolation,
  PreparedResolution,
  WorktreeRecord,
  WorktreeRecoveryScan,
  WorktreeRecoveryState,
} from "./model.ts";

export interface WorktreeIsolationOptions {
  rootDir?: string;
  credentialPath?: (path: string) => boolean;
  gitTimeoutMs?: number;
  lockTimeoutMs?: number;
}

export class WorktreeIsolation {
  private readonly store: WorktreeArtifactStore;
  private readonly git: GitClient;
  private readonly lockTimeoutMs: number;
  private readonly credentialPath: (path: string) => boolean;
  private readonly integrationTails = new Map<string, Promise<unknown>>();

  constructor(options: WorktreeIsolationOptions = {}) {
    const rootDir =
      options.rootDir ??
      join(
        process.env.NABLA_HOME ?? join(homedir(), ".nabla"),
        "worktrees",
      );
    this.credentialPath = options.credentialPath ?? (() => false);
    this.git = new GitClient(options.gitTimeoutMs);
    this.lockTimeoutMs = options.lockTimeoutMs ?? DEFAULT_LOCK_TIMEOUT_MS;
    this.store = new WorktreeArtifactStore(rootDir, this.credentialPath);
  }

  async prepare(
    agentId: string,
    cwd: string,
    policy: AgentIsolationPolicy,
    signal?: AbortSignal,
  ): Promise<PreparedIsolation> {
    if (policy.mode === "none") {
      return { backend: "shared", executionCwd: cwd };
    }
    const repository = await this.repositoryRoot(cwd, signal);
    if (!repository) {
      if (policy.mode === "worktree") {
        throw new Error(
          "This subagent requires worktree isolation, but the workspace is not a Git repository",
        );
      }
      return {
        backend: "shared_fallback",
        executionCwd: cwd,
        warning:
          "Worktree isolation is unavailable outside a Git repository; writable subagents are serialized in the shared workspace.",
      };
    }

    const originWorkspace = await realpath(cwd);
    const repoRoot = await realpath(repository);
    const relativeCwd = relative(repoRoot, originWorkspace);
    if (
      relativeCwd === ".." ||
      relativeCwd.startsWith(`..${sep}`) ||
      relativeCwd.startsWith(sep)
    ) {
      throw new Error("Workspace is outside the discovered Git repository");
    }
    const workspaceHash = sha256Hex(repoRoot).slice(0, 16);
    const id = `${agentId}-${randomUUID()}`;
    const artifactDirectory = join(this.store.rootDir, workspaceHash, id);
    const checkoutPath = join(artifactDirectory, "checkout");
    const patchPath = join(artifactDirectory, "result.patch");
    const indexPath = join(artifactDirectory, "baseline.index");
    await mkdir(artifactDirectory, { recursive: true, mode: 0o700 });

    let worktreeCreated = false;
    try {
      const head = await this.git.run(
        repoRoot,
        ["rev-parse", "--verify", "HEAD"],
        { allowFailure: true, signal },
      );
      const hadHead = head.code === 0;
      const indexEnv = { GIT_INDEX_FILE: indexPath };
      await this.git.run(
        repoRoot,
        hadHead ? ["read-tree", head.stdout.trim()] : ["read-tree", "--empty"],
        { env: indexEnv, signal },
      );
      await this.stageTracked(repoRoot, indexEnv, signal);
      const excludedPaths = await this.addSafeUntracked(
        repoRoot,
        indexEnv,
        signal,
      );
      const tree = await this.git.run(repoRoot, ["write-tree"], {
        env: indexEnv,
        signal,
      });
      const commitArgs = ["commit-tree", tree.stdout.trim()];
      if (hadHead) commitArgs.push("-p", head.stdout.trim());
      const baseline = await this.git.run(repoRoot, commitArgs, {
        env: { ...indexEnv, ...INTERNAL_GIT_IDENTITY },
        input: `Nabla baseline for ${agentId}\n`,
        signal,
      });
      const baselineCommit = baseline.stdout.trim();
      await this.git.run(
        repoRoot,
        ["worktree", "add", "--detach", checkoutPath, baselineCommit],
        { signal },
      );
      worktreeCreated = true;
      const now = new Date().toISOString();
      const record: WorktreeRecord = {
        schemaVersion: 2,
        id,
        agentId,
        originWorkspace,
        repoRoot,
        relativeCwd,
        checkoutPath,
        artifactDirectory,
        patchPath,
        baselineCommit,
        hadHead,
        backend: "worktree",
        integrationStatus: "none",
        changedPaths: [],
        patchBytes: 0,
        patchHash: this.store.patchHash(""),
        resolutionAttempts: 0,
        excludedPaths,
        createdAt: now,
        updatedAt: now,
      };
      await this.store.persist(record);
      const missingDependencies = await this.missingIgnoredDependencies(
        repoRoot,
        originWorkspace,
        checkoutPath,
      );
      const warnings = [
        ...(excludedPaths.length > 0
          ? [
              `Excluded ${excludedPaths.length} credential-like untracked path(s) from the worktree baseline.`,
            ]
          : []),
        ...(missingDependencies.length > 0
          ? [
              `Ignored dependency directories are not shared with the isolated worktree (${missingDependencies.join(", ")}); install or bootstrap them inside the worktree before verification if required.`,
            ]
          : []),
      ];
      return {
        backend: "worktree",
        executionCwd: resolve(checkoutPath, relativeCwd),
        record,
        ...(warnings.length > 0 ? { warning: warnings.join(" ") } : {}),
      };
    } catch (error) {
      if (worktreeCreated) {
        await this.removeRegisteredCheckout(repoRoot, checkoutPath).catch(
          () => undefined,
        );
      }
      throw error;
    } finally {
      await rm(indexPath, { force: true }).catch(() => undefined);
    }
  }

  async capture(
    record: WorktreeRecord,
    signal?: AbortSignal,
  ): Promise<CapturedWorktree> {
    const indexPath = join(record.artifactDirectory, "capture.index");
    const indexEnv = { GIT_INDEX_FILE: indexPath };
    try {
      await this.git.run(
        record.checkoutPath,
        ["read-tree", record.baselineCommit],
        { env: indexEnv, signal },
      );
      await this.stageTracked(record.checkoutPath, indexEnv, signal);
      const excludedPaths = await this.addSafeUntracked(
        record.checkoutPath,
        indexEnv,
        signal,
      );
      const patch = await this.git.run(
        record.checkoutPath,
        [
          "diff",
          "--cached",
          "--binary",
          "--full-index",
          record.baselineCommit,
          "--",
        ],
        { env: indexEnv, signal },
      );
      const names = await this.git.run(
        record.checkoutPath,
        [
          "diff",
          "--cached",
          "--name-only",
          "-z",
          record.baselineCommit,
          "--",
        ],
        { env: indexEnv, signal },
      );
      const changedPaths = names.stdout
        .split("\0")
        .filter(Boolean);
      for (const path of changedPaths) assertWorkspaceRelativePath(path);
      await writeAtomicFile(record.patchPath, patch.stdout);
      record.changedPaths = [...new Set(changedPaths)];
      record.patchBytes = Buffer.byteLength(patch.stdout);
      record.patchHash = this.store.patchHash(patch.stdout);
      record.excludedPaths = [
        ...new Set([...record.excludedPaths, ...excludedPaths]),
      ];
      record.integrationStatus =
        record.patchBytes > 0 ? "pending" : "none";
      record.updatedAt = new Date().toISOString();
      await this.store.persist(record);
      return { record, hasChanges: record.patchBytes > 0 };
    } finally {
      await rm(indexPath, { force: true }).catch(() => undefined);
    }
  }

  async annotate(
    record: WorktreeRecord,
    recovery: WorktreeRecoveryState,
  ): Promise<WorktreeRecord> {
    record.recovery = structuredClone(recovery);
    record.updatedAt = new Date().toISOString();
    await this.store.persist(record);
    return record;
  }

  async listRecoverable(originWorkspace: string): Promise<WorktreeRecoveryScan> {
    return listRecoverable(this.store, originWorkspace);
  }

  async pruneTerminalArtifacts(
    originWorkspace: string,
    now = Date.now(),
    retentionMs?: number,
  ): Promise<number> {
    return pruneTerminalArtifacts(
      this.store,
      this,
      originWorkspace,
      now,
      retentionMs,
    );
  }

  integrate(
    record: WorktreeRecord,
    signal?: AbortSignal,
  ): Promise<IntegrationResult> {
    const queueKey = resolve(record.repoRoot);
    const previous = this.integrationTails.get(queueKey) ?? Promise.resolve();
    const run = () =>
      this.withIntegrationLock(record, signal, async () => {
        const current = await this.store.loadRecord(record);
        if (current.integrationStatus === "applied") {
          return this.appliedResult(current);
        }
        if (current.integrationStatus === "discarded") {
          throw new Error(`Worktree result ${current.id} was already discarded`);
        }
        const patch = await readFile(current.patchPath, "utf8");
        const patchHash = this.store.patchHash(patch);
        if (current.patchHash !== patchHash) {
          return this.markReconciliation(
            current,
            "The captured patch changed after its record was persisted",
          );
        }
        if (!patch.trim()) {
          current.integrationStatus = "applied";
          current.applyStartedAt = undefined;
          current.updatedAt = new Date().toISOString();
          await this.store.persist(current);
          return this.appliedResult(current);
        }
        const check = await this.git.run(
          current.repoRoot,
          ["apply", "--check", "--binary", current.patchPath],
          { allowFailure: true, signal },
        );
        if (check.code !== 0) {
          const reverse = await this.git.run(
            current.repoRoot,
            ["apply", "--check", "--reverse", "--binary", current.patchPath],
            { allowFailure: true, signal },
          );
          if (current.integrationStatus === "applying" && reverse.code === 0) {
            current.integrationStatus = "applied";
            current.applyStartedAt = undefined;
            current.updatedAt = new Date().toISOString();
            await this.store.persist(current);
            return this.appliedResult(current);
          }
          if (current.integrationStatus === "applying") {
            return this.markReconciliation(
              current,
              (check.stderr || check.stdout).trim() ||
                "Interrupted patch application could not be reconciled",
            );
          }
          current.integrationStatus = "conflicted";
          current.updatedAt = new Date().toISOString();
          await this.store.persist(current);
          return {
            status: "conflicted",
            record: current,
            error: (check.stderr || check.stdout).trim() || "Patch conflicts",
          } satisfies IntegrationResult;
        }
        current.integrationStatus = "applying";
        current.applyStartedAt = new Date().toISOString();
        current.updatedAt = current.applyStartedAt;
        await this.store.persist(current);
        const applied = await this.git.run(
          current.repoRoot,
          ["apply", "--binary", current.patchPath],
          { allowFailure: true, signal },
        );
        if (applied.code !== 0) {
          return this.markReconciliation(
            current,
            (applied.stderr || applied.stdout).trim() || "Patch application failed",
          );
        }
        current.integrationStatus = "applied";
        current.applyStartedAt = undefined;
        current.updatedAt = new Date().toISOString();
        await this.store.persist(current);
        return this.appliedResult(current);
      });
    const result = previous.then(run, run);
    const tail = result.catch(() => undefined);
    this.integrationTails.set(queueKey, tail);
    void result.finally(() => {
      if (this.integrationTails.get(queueKey) === tail) {
        this.integrationTails.delete(queueKey);
      }
    }).catch(() => undefined);
    return result;
  }

  async prepareResolution(
    agentId: string,
    source: WorktreeRecord,
    signal?: AbortSignal,
  ): Promise<PreparedResolution> {
    if ((source.resolutionAttempts ?? 0) >= 1) {
      throw new Error(
        `The isolated conflict resolver has already been used for ${source.agentId}`,
      );
    }
    source.resolutionAttempts = (source.resolutionAttempts ?? 0) + 1;
    source.updatedAt = new Date().toISOString();
    await this.store.persist(source);
    const isolation = await this.prepare(
      agentId,
      source.originWorkspace,
      { mode: "worktree", integration: "auto" },
      signal,
    );
    if (!isolation.record) {
      throw new Error("Unable to create an integration worktree");
    }
    const applied = await this.git.run(
      isolation.record.checkoutPath,
      ["apply", "--3way", "--index", source.patchPath],
      { allowFailure: true, signal },
    );
    const conflicts = await this.git.run(
      isolation.record.checkoutPath,
      ["diff", "--name-only", "--diff-filter=U", "-z"],
      { allowFailure: true, signal },
    );
    return {
      isolation: isolation as PreparedIsolation & { record: WorktreeRecord },
      conflictPaths: conflicts.stdout.split("\0").filter(Boolean),
      ...((applied.stderr || applied.stdout).trim()
        ? { diagnostic: (applied.stderr || applied.stdout).trim() }
        : {}),
    };
  }

  async resolvedBy(
    source: WorktreeRecord,
    resolverId: string,
  ): Promise<WorktreeRecord> {
    await this.cleanupCheckout(source);
    source.integrationStatus = "applied";
    source.updatedAt = new Date().toISOString();
    await this.store.persist(source);
    const path = join(source.artifactDirectory, "resolved-by");
    await writeFile(path, `${resolverId}\n`, { encoding: "utf8", mode: 0o600 });
    return source;
  }

  async assertResolved(record: WorktreeRecord): Promise<void> {
    const unresolved: string[] = [];
    for (const path of record.changedPaths) {
      const content = await readFile(join(record.checkoutPath, path)).catch(
        () => undefined,
      );
      if (
        content &&
        /^(?:<{7}|={7}|>{7})(?: .*)?$/mu.test(content.toString("utf8"))
      ) {
        unresolved.push(path);
      }
    }
    if (unresolved.length > 0) {
      throw new Error(
        `Conflict resolver left merge markers in: ${unresolved.join(", ")}`,
      );
    }
  }

  async keep(record: WorktreeRecord): Promise<WorktreeRecord> {
    record.integrationStatus = "kept";
    record.updatedAt = new Date().toISOString();
    await this.store.persist(record);
    return record;
  }

  async discard(record: WorktreeRecord): Promise<WorktreeRecord> {
    await this.cleanupCheckout(record);
    await rm(record.patchPath, { force: true });
    record.integrationStatus = "discarded";
    record.patchBytes = 0;
    record.updatedAt = new Date().toISOString();
    await this.store.persist(record);
    return record;
  }

  async cleanupCheckout(record: WorktreeRecord): Promise<void> {
    await this.removeRegisteredCheckout(record.repoRoot, record.checkoutPath);
  }

  private async repositoryRoot(
    cwd: string,
    signal?: AbortSignal,
  ): Promise<string | undefined> {
    let result: GitResult;
    try {
      result = await this.git.run(cwd, ["rev-parse", "--show-toplevel"], {
        allowFailure: true,
        signal,
      });
    } catch (error) {
      if (
        error instanceof Error &&
        "code" in error &&
        (error as NodeJS.ErrnoException).code === "ENOENT"
      ) {
        throw new Error("Git executable is unavailable");
      }
      throw error;
    }
    if (result.code === 0) return result.stdout.trim();
    const diagnostic = `${result.stdout}\n${result.stderr}`;
    if (/not a git repository/iu.test(diagnostic)) return undefined;
    throw new Error(
      diagnostic.trim() || "Unable to inspect the Git repository",
    );
  }

  private async addSafeUntracked(
    cwd: string,
    env: NodeJS.ProcessEnv,
    signal?: AbortSignal,
  ): Promise<string[]> {
    const result = await this.git.run(
      cwd,
      ["ls-files", "--others", "--exclude-standard", "-z"],
      { env, signal },
    );
    const paths = result.stdout.split("\0").filter(Boolean);
    const excluded = paths.filter((path) =>
      this.credentialPath(resolve(cwd, path)),
    );
    const included = paths.filter((path) => !excluded.includes(path));
    for (let offset = 0; offset < included.length; offset += 128) {
      await this.git.run(
        cwd,
        ["add", "--", ...included.slice(offset, offset + 128)],
        { env, signal },
      );
    }
    return excluded;
  }

  private async missingIgnoredDependencies(
    repoRoot: string,
    originWorkspace: string,
    checkoutPath: string,
  ): Promise<string[]> {
    const names = ["node_modules", ".venv", "venv"];
    const missing: string[] = [];
    let directory = originWorkspace;
    while (isPathWithin(repoRoot, directory)) {
      for (const name of names) {
        const source = join(directory, name);
        const sourceMetadata = await stat(source).catch(() => undefined);
        if (!sourceMetadata?.isDirectory()) continue;
        const repoRelative = relative(repoRoot, source);
        const checkoutMetadata = await stat(
          resolve(checkoutPath, repoRelative),
        ).catch(() => undefined);
        if (!checkoutMetadata?.isDirectory()) {
          missing.push(repoRelative.replace(/\\/gu, "/"));
        }
      }
      if (directory === repoRoot) break;
      const parent = dirname(directory);
      if (parent === directory) break;
      directory = parent;
    }
    return [...new Set(missing)].sort();
  }

  private async stageTracked(
    cwd: string,
    env: NodeJS.ProcessEnv,
    signal?: AbortSignal,
  ): Promise<void> {
    const result = await this.git.run(cwd, ["add", "-u", "--", "."], {
      allowFailure: true,
      env,
      signal,
    });
    if (
      result.code !== 0 &&
      !/pathspec .* did not match any file/iu.test(
        `${result.stdout}\n${result.stderr}`,
      )
    ) {
      throw new Error(
        result.stderr.trim() ||
          result.stdout.trim() ||
          "Unable to stage tracked worktree state",
      );
    }
  }

  private async removeRegisteredCheckout(
    repoRoot: string,
    checkoutPath: string,
  ): Promise<void> {
    const managedRoot = resolve(dirname(dirname(checkoutPath)));
    const target = resolve(checkoutPath);
    if (!isPathWithin(managedRoot, target)) {
      throw new Error("Refusing to remove an unmanaged worktree path");
    }
    const listed = await this.git.run(
      repoRoot,
      ["worktree", "list", "--porcelain", "-z"],
      { allowFailure: true },
    );
    const normalizedTarget = await realpath(target).catch(() => target);
    const registered = listed.stdout
      .split("\0")
      .filter((line) => line.startsWith("worktree "))
      .map((line) => line.slice("worktree ".length))
      .some(
        (path) =>
          resolve(path) === target ||
          resolve(path) === resolve(normalizedTarget),
      );
    if (!registered) return;
    await this.git.run(
      repoRoot,
      ["worktree", "remove", "--force", checkoutPath],
      {},
    );
  }

  private async markReconciliation(
    record: WorktreeRecord,
    error: string,
  ): Promise<IntegrationResult> {
    record.integrationStatus = "needs_reconciliation";
    record.updatedAt = new Date().toISOString();
    await this.store.persist(record);
    return { status: "needs_reconciliation", record, error };
  }

  private async appliedResult(
    record: WorktreeRecord,
  ): Promise<IntegrationResult> {
    try {
      await this.cleanupCheckout(record);
      return { status: "applied", record };
    } catch (error) {
      return {
        status: "applied",
        record,
        error: `Patch was applied, but the managed checkout could not be cleaned up: ${
          errorMessage(error)
        }`,
      };
    }
  }

  async withIntegrationLock<T>(
    record: WorktreeRecord,
    signal: AbortSignal | undefined,
    action: () => Promise<T>,
  ): Promise<T> {
    const workspaceRoot = dirname(record.artifactDirectory);
    if (!isPathWithin(this.store.rootDir, workspaceRoot)) {
      throw new Error("Refusing to lock an unmanaged worktree directory");
    }
    await mkdir(workspaceRoot, { recursive: true, mode: 0o700 });
    const lockPath = join(workspaceRoot, ".integration.lock");
    const deadline = Date.now() + this.lockTimeoutMs;
    while (true) {
      if (signal?.aborted) throw signal.reason ?? new Error("Integration aborted");
      try {
        await mkdir(lockPath, { mode: 0o700 });
        await writeAtomicJson(join(lockPath, "owner.json"), {
          pid: process.pid,
          recordId: record.id,
          acquiredAt: new Date().toISOString(),
        });
        break;
      } catch (error) {
        const code =
          error && typeof error === "object" && "code" in error
            ? String(error.code)
            : "";
        if (code !== "EEXIST") throw error;
        if (await this.integrationLockIsStale(lockPath)) {
          await rm(lockPath, { recursive: true, force: true });
          continue;
        }
        if (Date.now() >= deadline) {
          throw new Error(`Timed out waiting to integrate worktree ${record.id}`);
        }
        await new Promise<void>((resolvePromise, reject) => {
          const finish = () => {
            signal?.removeEventListener("abort", abort);
            resolvePromise();
          };
          const timer = setTimeout(finish, 25);
          const abort = () => {
            clearTimeout(timer);
            signal?.removeEventListener("abort", abort);
            reject(signal?.reason ?? new Error("Integration aborted"));
          };
          signal?.addEventListener("abort", abort, { once: true });
          if (signal?.aborted) abort();
        });
      }
    }
    try {
      return await action();
    } finally {
      await rm(lockPath, { recursive: true, force: true });
    }
  }

  private async integrationLockIsStale(lockPath: string): Promise<boolean> {
    const owner = await readFile(join(lockPath, "owner.json"), "utf8")
      .then((content) => JSON.parse(content) as unknown)
      .catch(() => undefined);
    if (owner && typeof owner === "object" && !Array.isArray(owner)) {
      const pid = (owner as { pid?: unknown }).pid;
      if (typeof pid === "number" && Number.isInteger(pid) && pid > 0) {
        try {
          process.kill(pid, 0);
          return false;
        } catch (error) {
          const code =
            error && typeof error === "object" && "code" in error
              ? String(error.code)
              : "";
          return code === "ESRCH";
        }
      }
    }
    const metadata = await stat(lockPath).catch(() => undefined);
    return metadata ? Date.now() - metadata.mtimeMs > this.lockTimeoutMs : true;
  }
}
