import type { Dirent } from "node:fs";
import { mkdir, readdir, readFile, realpath } from "node:fs/promises";
import { join, relative, resolve, sep } from "node:path";

import { writeAtomicJson } from "../../../persistence/atomic-json.ts";
import { assertWorkspaceRelativePath } from "../../permissions/filesystem/path.ts";
import { sha256Hex } from "../../permissions/shell/digest.ts";
import type {
  WorktreeRecord,
  WorktreeRecoveryScan,
} from "./model.ts";

export class WorktreeArtifactStore {
  readonly rootDir: string;
  private readonly credentialPath: (path: string) => boolean;

  constructor(
    rootDir: string,
    credentialPath: (path: string) => boolean,
  ) {
    this.rootDir = rootDir;
    this.credentialPath = credentialPath;
  }

  patchHash(patch: string): string {
    return sha256Hex(patch);
  }

  async persist(record: WorktreeRecord): Promise<void> {
    await mkdir(record.artifactDirectory, { recursive: true, mode: 0o700 });
    const path = join(record.artifactDirectory, "record.json");
    await writeAtomicJson(path, record);
  }

  async loadRecord(record: WorktreeRecord): Promise<WorktreeRecord> {
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

  async scanManagedRecords(): Promise<WorktreeRecoveryScan> {
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
}
