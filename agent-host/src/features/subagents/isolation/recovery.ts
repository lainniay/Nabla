import { readdir, realpath, rm } from "node:fs/promises";
import { dirname, resolve } from "node:path";

import type { WorktreeArtifactStore } from "./artifact-store.ts";
import {
  DEFAULT_TERMINAL_RETENTION_MS,
  type WorktreeRecord,
  type WorktreeRecoveryScan,
} from "./model.ts";

export { DEFAULT_TERMINAL_RETENTION_MS } from "./model.ts";

export interface TerminalPrunePort {
  cleanupCheckout(record: WorktreeRecord): Promise<void>;
  withIntegrationLock<T>(
    record: WorktreeRecord,
    signal: AbortSignal | undefined,
    action: () => Promise<T>,
  ): Promise<T>;
}

export async function listRecoverable(
  store: WorktreeArtifactStore,
  originWorkspace: string,
): Promise<WorktreeRecoveryScan> {
  const expectedWorkspace = await realpath(originWorkspace).catch(() =>
    resolve(originWorkspace),
  );
  const scan = await store.scanManagedRecords();
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

export async function pruneTerminalArtifacts(
  store: WorktreeArtifactStore,
  port: TerminalPrunePort,
  originWorkspace: string,
  now = Date.now(),
  retentionMs = DEFAULT_TERMINAL_RETENTION_MS,
): Promise<number> {
  const expectedWorkspace = await realpath(originWorkspace).catch(() =>
    resolve(originWorkspace),
  );
  let removed = 0;
  const scan = await store.scanManagedRecords();
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
    await port.withIntegrationLock(record, undefined, async () => {
      const current = await store.loadRecord(record);
      const currentUpdatedAt = Date.parse(current.updatedAt);
      if (
        (current.integrationStatus !== "applied" &&
          current.integrationStatus !== "discarded") ||
        !Number.isFinite(currentUpdatedAt) ||
        now - currentUpdatedAt < Math.max(0, retentionMs)
      ) {
        return;
      }
      await port.cleanupCheckout(current);
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
