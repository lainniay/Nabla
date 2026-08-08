import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import type { HarnessConfig } from "../workspace/config.ts";
import { EMPTY_SANDBOX_CONFIG } from "../permissions/execution/sandbox-config.ts";
import type { WorktreeRecord } from "./isolation/model.ts";
import { IntegrationService } from "./isolation/integration-service.ts";

const config: HarnessConfig = {
  schemaVersion: 2,
  maxParallel: 2,
  trustedWorkspaces: [],
  allowedProjectExtensions: [],
  profiles: {
    worker: {
      description: "Worker",
      source: "builtin",
      skills: [],
      tools: ["read", "edit"],
      permission: {},
      maxParallel: 1,
      maxTurns: 10,
      isolation: { mode: "auto", integration: "auto" },
      disabled: false,
      instructions: [],
    },
  },
  sandbox: EMPTY_SANDBOX_CONFIG,
  diagnostics: [],
};

function record(root: string, changedPaths: string[]): WorktreeRecord {
  return {
    id: "record-1",
    agentId: "agent-1",
    repoRoot: root,
    checkoutPath: root,
    originWorkspace: root,
    changedPaths,
    excludedPaths: [],
    patchBytes: 0,
    createdAt: "2026-01-01T00:00:00.000Z",
    updatedAt: "2026-01-01T00:00:00.000Z",
    integrationStatus: "pending",
  } as unknown as WorktreeRecord;
}

test("recover on a non-git directory returns nothing without warnings", async () => {
  const root = mkdtempSync(join(tmpdir(), "nabla-integration-"));
  try {
    const warnings: string[] = [];
    const service = new IntegrationService(
      (message) => warnings.push(message),
      () => config,
    );
    const recovered = await service.recover(root);
    assert.deepEqual(recovered, []);
    assert.deepEqual(warnings, []);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("validateWorktreePaths rejects credential-like changes", () => {
  const root = mkdtempSync(join(tmpdir(), "nabla-integration-"));
  try {
    const service = new IntegrationService(() => {}, () => config);
    assert.throws(
      () =>
        service.validateWorktreePaths(
          record(root, [".ssh/id_rsa"]),
          config.profiles.worker!,
          root,
        ),
      /credential-like path/u,
    );
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("validateWorktreePaths rejects outside-workspace changes", () => {
  const root = mkdtempSync(join(tmpdir(), "nabla-integration-"));
  try {
    const service = new IntegrationService(() => {}, () => config);
    assert.throws(
      () =>
        service.validateWorktreePaths(
          record(root, ["../outside.txt"]),
          config.profiles.worker!,
          root,
        ),
      /outside the workspace/u,
    );
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("validateWorktreePaths accepts allowed workspace changes", () => {
  const root = mkdtempSync(join(tmpdir(), "nabla-integration-"));
  try {
    const service = new IntegrationService(() => {}, () => config);
    assert.doesNotThrow(() =>
      service.validateWorktreePaths(
        record(root, ["src/a.ts"]),
        config.profiles.worker!,
        root,
      ),
    );
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
