import { createHash, randomUUID } from "node:crypto";
import { spawn } from "node:child_process";
import type { Dirent } from "node:fs";
import {
  mkdir,
  readdir,
  readFile,
  realpath,
  rm,
  stat,
  writeFile,
} from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join, relative, resolve, sep } from "node:path";

import { writeAtomicFile, writeAtomicJson } from "./persistence/atomic-json.ts";
import {
  assertWorkspaceRelativePath,
  isPathWithin,
} from "./policy/path-boundary.ts";

export type IsolationMode = "none" | "auto" | "worktree";
export type IntegrationMode = "source" | "auto" | "ask" | "manual";
export type IsolationBackend = "shared" | "shared_fallback" | "worktree";
export type IntegrationStatus =
  | "none"
  | "pending"
  | "applying"
  | "applied"
  | "kept"
  | "conflicted"
  | "needs_reconciliation"
  | "discarded";

export interface AgentIsolationPolicy {
  mode: IsolationMode;
  integration: IntegrationMode;
}

export interface WorktreeRecoveryState {
  profile: string;
  task: string;
  direct: boolean;
  planReadOnly: boolean;
  model: string;
  originSessionId: string;
  result?: Record<string, unknown>;
}

export interface WorktreeRecord {
  schemaVersion: 2;
  id: string;
  agentId: string;
  originWorkspace: string;
  repoRoot: string;
  relativeCwd: string;
  checkoutPath: string;
  artifactDirectory: string;
  patchPath: string;
  baselineCommit: string;
  hadHead: boolean;
  backend: "worktree";
  integrationStatus: IntegrationStatus;
  changedPaths: string[];
  patchBytes: number;
  patchHash: string;
  applyStartedAt?: string;
  resolutionAttempts?: number;
  excludedPaths: string[];
  createdAt: string;
  updatedAt: string;
  recovery?: WorktreeRecoveryState;
}

export interface PreparedIsolation {
  backend: IsolationBackend;
  executionCwd: string;
  warning?: string;
  record?: WorktreeRecord;
}

export interface CapturedWorktree {
  record: WorktreeRecord;
  hasChanges: boolean;
}

export interface IntegrationResult {
  status: "applied" | "conflicted" | "needs_reconciliation";
  record: WorktreeRecord;
  error?: string;
}

export interface WorktreeRecoveryScan {
  records: WorktreeRecord[];
  warnings: string[];
}

export interface PreparedResolution {
  isolation: PreparedIsolation & { record: WorktreeRecord };
  conflictPaths: string[];
  diagnostic?: string;
}

interface WorktreeManagerOptions {
  rootDir?: string;
  credentialPath?: (path: string) => boolean;
  gitTimeoutMs?: number;
  lockTimeoutMs?: number;
}

interface GitResult {
  code: number;
  stdout: string;
  stderr: string;
}

const DEFAULT_GIT_TIMEOUT_MS = 30_000;
const DEFAULT_LOCK_TIMEOUT_MS = 60_000;
const DEFAULT_TERMINAL_RETENTION_MS = 30 * 24 * 60 * 60 * 1_000;
const INTERNAL_GIT_IDENTITY = {
  GIT_AUTHOR_NAME: "Nabla",
  GIT_AUTHOR_EMAIL: "nabla@local",
  GIT_COMMITTER_NAME: "Nabla",
  GIT_COMMITTER_EMAIL: "nabla@local",
};

export class WorktreeManager {
  private readonly rootDir: string;
  private readonly credentialPath: (path: string) => boolean;
  private readonly gitTimeoutMs: number;
  private readonly lockTimeoutMs: number;
  private readonly integrationTails = new Map<string, Promise<unknown>>();

  constructor(options: WorktreeManagerOptions = {}) {
    this.rootDir =
      options.rootDir ??
      join(
        process.env.NABLA_HOME ?? join(homedir(), ".nabla"),
        "worktrees",
      );
    this.credentialPath = options.credentialPath ?? (() => false);
    this.gitTimeoutMs = options.gitTimeoutMs ?? DEFAULT_GIT_TIMEOUT_MS;
    this.lockTimeoutMs = options.lockTimeoutMs ?? DEFAULT_LOCK_TIMEOUT_MS;
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
    const workspaceHash = createHash("sha256")
      .update(repoRoot)
      .digest("hex")
      .slice(0, 16);
    const id = `${agentId}-${randomUUID()}`;
    const artifactDirectory = join(this.rootDir, workspaceHash, id);
    const checkoutPath = join(artifactDirectory, "checkout");
    const patchPath = join(artifactDirectory, "result.patch");
    const indexPath = join(artifactDirectory, "baseline.index");
    await mkdir(artifactDirectory, { recursive: true, mode: 0o700 });

    let worktreeCreated = false;
    try {
      const head = await this.git(repoRoot, ["rev-parse", "--verify", "HEAD"], {
        allowFailure: true,
        signal,
      });
      const hadHead = head.code === 0;
      const indexEnv = { GIT_INDEX_FILE: indexPath };
      await this.git(
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
      const tree = await this.git(repoRoot, ["write-tree"], {
        env: indexEnv,
        signal,
      });
      const commitArgs = ["commit-tree", tree.stdout.trim()];
      if (hadHead) commitArgs.push("-p", head.stdout.trim());
      const baseline = await this.git(repoRoot, commitArgs, {
        env: { ...indexEnv, ...INTERNAL_GIT_IDENTITY },
        input: `Nabla baseline for ${agentId}\n`,
        signal,
      });
      const baselineCommit = baseline.stdout.trim();
      await this.git(
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
        patchHash: this.patchHash(""),
        resolutionAttempts: 0,
        excludedPaths,
        createdAt: now,
        updatedAt: now,
      };
      await this.persist(record);
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
      await this.git(
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
      const patch = await this.git(
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
      const names = await this.git(
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
      record.patchHash = this.patchHash(patch.stdout);
      record.excludedPaths = [
        ...new Set([...record.excludedPaths, ...excludedPaths]),
      ];
      record.integrationStatus =
        record.patchBytes > 0 ? "pending" : "none";
      record.updatedAt = new Date().toISOString();
      await this.persist(record);
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
    await this.persist(record);
    return record;
  }

  async listRecoverable(originWorkspace: string): Promise<WorktreeRecoveryScan> {
    const expectedWorkspace = await realpath(originWorkspace).catch(() =>
      resolve(originWorkspace),
    );
    const scan = await this.scanManagedRecords();
    const records: WorktreeRecord[] = [];
    for (const record of scan.records) {
      if (
        record.integrationStatus === "applied" ||
        record.integrationStatus === "discarded" ||
        record.integrationStatus === "kept"
      ) {
        continue;
      }
      const recordWorkspace = await realpath(record.originWorkspace).catch(
        () => resolve(record.originWorkspace),
      );
      if (recordWorkspace !== expectedWorkspace) continue;
      records.push(record);
    }
    return {
      records: records.sort((left, right) =>
        left.createdAt.localeCompare(right.createdAt),
      ),
      warnings: scan.warnings,
    };
  }

  async pruneTerminalArtifacts(
    originWorkspace: string,
    now = Date.now(),
    retentionMs = DEFAULT_TERMINAL_RETENTION_MS,
  ): Promise<number> {
    const expectedWorkspace = await realpath(originWorkspace).catch(() =>
      resolve(originWorkspace),
    );
    let removed = 0;
    const scan = await this.scanManagedRecords();
    const failures: unknown[] = scan.warnings.map((warning) => new Error(warning));
    const affectedWorkspacePaths = new Set<string>();
    for (const record of scan.records) {
      const recordWorkspace = await realpath(record.originWorkspace).catch(
        () => resolve(record.originWorkspace),
      );
      const updatedAt = Date.parse(record.updatedAt);
      if (
        recordWorkspace !== expectedWorkspace ||
        (record.integrationStatus !== "applied" &&
          record.integrationStatus !== "discarded") ||
        !Number.isFinite(updatedAt) ||
        now - updatedAt < Math.max(0, retentionMs)
      ) {
        continue;
      }
      await this.withIntegrationLock(record, undefined, async () => {
        const current = await this.loadRecord(record);
        const currentUpdatedAt = Date.parse(current.updatedAt);
        if (
          (current.integrationStatus !== "applied" &&
            current.integrationStatus !== "discarded") ||
          !Number.isFinite(currentUpdatedAt) ||
          now - currentUpdatedAt < Math.max(0, retentionMs)
        ) {
          return;
        }
        await this.cleanupCheckout(current);
        await rm(current.artifactDirectory, { recursive: true, force: true });
        affectedWorkspacePaths.add(dirname(current.artifactDirectory));
        removed += 1;
      }).catch((error) => failures.push(error));
    }
    for (const workspacePath of affectedWorkspacePaths) {
      try {
        const remaining = await readdir(workspacePath);
        if (remaining.length === 0) {
          await rm(workspacePath, { recursive: true, force: true });
        }
      } catch (error) {
        const code =
          error && typeof error === "object" && "code" in error
            ? String(error.code)
            : "";
        if (code !== "ENOENT") failures.push(error);
      }
    }
    if (failures.length > 0) {
      throw new AggregateError(
        failures,
        `Failed to prune ${failures.length} terminal worktree artifact(s)`,
      );
    }
    return removed;
  }

  integrate(
    record: WorktreeRecord,
    signal?: AbortSignal,
  ): Promise<IntegrationResult> {
    const queueKey = resolve(record.repoRoot);
    const previous = this.integrationTails.get(queueKey) ?? Promise.resolve();
    const run = () =>
      this.withIntegrationLock(record, signal, async () => {
      const current = await this.loadRecord(record);
      if (current.integrationStatus === "applied") {
        return this.appliedResult(current);
      }
      if (current.integrationStatus === "discarded") {
        throw new Error(`Worktree result ${current.id} was already discarded`);
      }
      const patch = await readFile(current.patchPath, "utf8");
      const patchHash = this.patchHash(patch);
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
        await this.persist(current);
        return this.appliedResult(current);
      }
      const check = await this.git(
        current.repoRoot,
        ["apply", "--check", "--binary", current.patchPath],
        { allowFailure: true, signal },
      );
      if (check.code !== 0) {
        const reverse = await this.git(
          current.repoRoot,
          ["apply", "--check", "--reverse", "--binary", current.patchPath],
          { allowFailure: true, signal },
        );
        if (current.integrationStatus === "applying" && reverse.code === 0) {
          current.integrationStatus = "applied";
          current.applyStartedAt = undefined;
          current.updatedAt = new Date().toISOString();
          await this.persist(current);
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
        await this.persist(current);
        return {
          status: "conflicted",
          record: current,
          error: (check.stderr || check.stdout).trim() || "Patch conflicts",
        } satisfies IntegrationResult;
      }
      current.integrationStatus = "applying";
      current.applyStartedAt = new Date().toISOString();
      current.updatedAt = current.applyStartedAt;
      await this.persist(current);
      const applied = await this.git(
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
      await this.persist(current);
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
    await this.persist(source);
    const isolation = await this.prepare(
      agentId,
      source.originWorkspace,
      { mode: "worktree", integration: "auto" },
      signal,
    );
    if (!isolation.record) {
      throw new Error("Unable to create an integration worktree");
    }
    const applied = await this.git(
      isolation.record.checkoutPath,
      ["apply", "--3way", "--index", source.patchPath],
      { allowFailure: true, signal },
    );
    const conflicts = await this.git(
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
    await this.persist(source);
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
    await this.persist(record);
    return record;
  }

  async discard(record: WorktreeRecord): Promise<WorktreeRecord> {
    await this.cleanupCheckout(record);
    await rm(record.patchPath, { force: true });
    record.integrationStatus = "discarded";
    record.patchBytes = 0;
    record.updatedAt = new Date().toISOString();
    await this.persist(record);
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
      result = await this.git(cwd, ["rev-parse", "--show-toplevel"], {
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
    const result = await this.git(
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
      await this.git(
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
    const result = await this.git(cwd, ["add", "-u", "--", "."], {
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
    const listed = await this.git(
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
    await this.git(
      repoRoot,
      ["worktree", "remove", "--force", checkoutPath],
      {},
    );
  }

  private async persist(record: WorktreeRecord): Promise<void> {
    await mkdir(record.artifactDirectory, { recursive: true, mode: 0o700 });
    const path = join(record.artifactDirectory, "record.json");
    await writeAtomicJson(path, record);
  }

  private async loadRecord(record: WorktreeRecord): Promise<WorktreeRecord> {
    const path = join(record.artifactDirectory, "record.json");
    const parsed = JSON.parse(await readFile(path, "utf8")) as unknown;
    const current = await this.recoverableRecord(
      parsed,
      record.artifactDirectory,
    );
    if (!current || current.id !== record.id) {
      throw new Error(`Invalid persisted worktree record: ${record.id}`);
    }
    return current;
  }

  private async recoverableRecord(
    value: unknown,
    artifactPath: string,
  ): Promise<WorktreeRecord | undefined> {
    if (!value || typeof value !== "object" || Array.isArray(value)) {
      return undefined;
    }
    const candidate = value as Omit<Partial<WorktreeRecord>, "schemaVersion"> & {
      schemaVersion?: number;
      patchHash?: string;
    };
    const strings = [
      candidate.id,
      candidate.agentId,
      candidate.originWorkspace,
      candidate.repoRoot,
      candidate.relativeCwd,
      candidate.checkoutPath,
      candidate.artifactDirectory,
      candidate.patchPath,
      candidate.baselineCommit,
      candidate.createdAt,
      candidate.updatedAt,
    ];
    if (
      (candidate.schemaVersion !== 1 && candidate.schemaVersion !== 2) ||
      candidate.backend !== "worktree" ||
      strings.some((field) => typeof field !== "string") ||
      !Array.isArray(candidate.changedPaths) ||
      !candidate.changedPaths.every((path) => typeof path === "string") ||
      !Array.isArray(candidate.excludedPaths) ||
      !candidate.excludedPaths.every((path) => typeof path === "string") ||
      typeof candidate.patchBytes !== "number" ||
      (candidate.schemaVersion === 2 && typeof candidate.patchHash !== "string") ||
      (candidate.resolutionAttempts !== undefined &&
        (!Number.isInteger(candidate.resolutionAttempts) ||
          candidate.resolutionAttempts < 0)) ||
      typeof candidate.hadHead !== "boolean"
    ) {
      return undefined;
    }
    const status = candidate.integrationStatus;
    if (
      status !== "none" &&
      status !== "pending" &&
      status !== "applying" &&
      status !== "conflicted" &&
      status !== "needs_reconciliation" &&
      status !== "kept" &&
      status !== "applied" &&
      status !== "discarded"
    ) {
      return undefined;
    }
    if (
      resolve(candidate.artifactDirectory!) !== resolve(artifactPath) ||
      resolve(candidate.checkoutPath!) !== resolve(artifactPath, "checkout") ||
      resolve(candidate.patchPath!) !== resolve(artifactPath, "result.patch")
    ) {
      return undefined;
    }
    const normalizedRepoRoot = await realpath(candidate.repoRoot!).catch(() =>
      resolve(candidate.repoRoot!),
    );
    const normalizedOriginWorkspace = await realpath(
      candidate.originWorkspace!,
    ).catch(() => resolve(candidate.originWorkspace!));
    const expectedRelativeCwd = relative(
      normalizedRepoRoot,
      normalizedOriginWorkspace,
    );
    if (
      expectedRelativeCwd !== candidate.relativeCwd ||
      expectedRelativeCwd === ".." ||
      expectedRelativeCwd.startsWith(`..${sep}`) ||
      expectedRelativeCwd.startsWith(sep)
    ) {
      return undefined;
    }
    for (const path of [...candidate.changedPaths, ...candidate.excludedPaths]) {
      try {
        assertWorkspaceRelativePath(path);
      } catch {
        return undefined;
      }
    }
    if (candidate.schemaVersion === 1) {
      const patch = await readFile(candidate.patchPath!, "utf8");
      candidate.schemaVersion = 2;
      candidate.patchHash = this.patchHash(patch);
    }
    return candidate as WorktreeRecord;
  }

  private patchHash(patch: string): string {
    return createHash("sha256").update(patch).digest("hex");
  }

  private async scanManagedRecords(): Promise<WorktreeRecoveryScan> {
    const records: WorktreeRecord[] = [];
    const warnings: string[] = [];
    const workspaceDirectories = await this.readDirectories(
      this.rootDir,
      warnings,
    );
    for (const workspaceDirectory of workspaceDirectories) {
      if (!workspaceDirectory.isDirectory()) continue;
      const workspacePath = join(this.rootDir, workspaceDirectory.name);
      const artifactDirectories = await this.readDirectories(
        workspacePath,
        warnings,
      );
      for (const artifactDirectory of artifactDirectories) {
        if (
          !artifactDirectory.isDirectory() ||
          artifactDirectory.name.startsWith(".")
        ) {
          continue;
        }
        const artifactPath = join(workspacePath, artifactDirectory.name);
        const recordPath = join(artifactPath, "record.json");
        let parsed: unknown;
        try {
          parsed = JSON.parse(await readFile(recordPath, "utf8")) as unknown;
        } catch (error) {
          warnings.push(
            `Unable to read worktree recovery record ${recordPath}: ${
              error instanceof Error ? error.message : String(error)
            }`,
          );
          continue;
        }
        try {
          const record = await this.recoverableRecord(parsed, artifactPath);
          if (record) {
            records.push(record);
          } else {
            warnings.push(
              `Ignored invalid worktree recovery record ${recordPath}.`,
            );
          }
        } catch (error) {
          warnings.push(
            `Unable to validate worktree recovery record ${recordPath}: ${
              error instanceof Error ? error.message : String(error)
            }`,
          );
        }
      }
    }
    return { records, warnings };
  }

  private async readDirectories(
    path: string,
    warnings: string[],
  ): Promise<Dirent[]> {
    try {
      return await readdir(path, { withFileTypes: true });
    } catch (error) {
      const code =
        error && typeof error === "object" && "code" in error
          ? String(error.code)
          : "";
      if (code !== "ENOENT") {
        warnings.push(
          `Unable to scan managed worktree directory ${path}: ${
            error instanceof Error ? error.message : String(error)
          }`,
        );
      }
      return [];
    }
  }

  private async markReconciliation(
    record: WorktreeRecord,
    error: string,
  ): Promise<IntegrationResult> {
    record.integrationStatus = "needs_reconciliation";
    record.updatedAt = new Date().toISOString();
    await this.persist(record);
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
          error instanceof Error ? error.message : String(error)
        }`,
      };
    }
  }

  private async withIntegrationLock<T>(
    record: WorktreeRecord,
    signal: AbortSignal | undefined,
    action: () => Promise<T>,
  ): Promise<T> {
    const workspaceRoot = dirname(record.artifactDirectory);
    if (!isPathWithin(this.rootDir, workspaceRoot)) {
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

  private git(
    cwd: string,
    args: string[],
    options: {
      allowFailure?: boolean;
      env?: NodeJS.ProcessEnv;
      input?: string;
      signal?: AbortSignal;
    } = {},
  ): Promise<GitResult> {
    return new Promise((resolvePromise, reject) => {
      const child = spawn("git", ["-C", cwd, ...args], {
        cwd,
        env: { ...process.env, ...options.env },
        stdio: ["pipe", "pipe", "pipe"],
      });
      const stdout: Buffer[] = [];
      const stderr: Buffer[] = [];
      const timer = setTimeout(() => {
        child.kill("SIGTERM");
        reject(new Error(`Git command timed out: git ${args.join(" ")}`));
      }, this.gitTimeoutMs);
      const abort = () => child.kill("SIGTERM");
      options.signal?.addEventListener("abort", abort, { once: true });
      child.stdout.on("data", (chunk: Buffer) => stdout.push(chunk));
      child.stderr.on("data", (chunk: Buffer) => stderr.push(chunk));
      child.on("error", (error) => {
        clearTimeout(timer);
        options.signal?.removeEventListener("abort", abort);
        reject(error);
      });
      child.on("close", (code) => {
        clearTimeout(timer);
        options.signal?.removeEventListener("abort", abort);
        const result = {
          code: code ?? 1,
          stdout: Buffer.concat(stdout).toString("utf8"),
          stderr: Buffer.concat(stderr).toString("utf8"),
        };
        if (result.code !== 0 && !options.allowFailure) {
          reject(
            new Error(
              result.stderr.trim() ||
                result.stdout.trim() ||
                `Git command failed (${result.code}): git ${args.join(" ")}`,
            ),
          );
        } else {
          resolvePromise(result);
        }
      });
      if (options.input) child.stdin.end(options.input);
      else child.stdin.end();
    });
  }
}
