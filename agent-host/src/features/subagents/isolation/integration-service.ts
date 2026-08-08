import { resolve } from "node:path";

import { isCredentialPath } from "../../permissions/filesystem/credential.ts";
import type { AgentProfile } from "../profile-model.ts";
import type { HarnessConfig } from "../../workspace/config.ts";
import { evaluateProfilePermission } from "../../permissions/policy/profile-compiler.ts";
import { workspaceRelativePath } from "../../permissions/filesystem/path.ts";
import {
  errorMessage,
  isJsonObject,
} from "../../../protocol/validation.ts";
import type {
  AgentIsolationPolicy,
  WorktreeRecord,
  WorktreeRecoveryState,
} from "./model.ts";
import {
  WorktreeIsolation,
} from "./worktree.ts";

export interface RecoveredSubagent {
  record: WorktreeRecord;
  metadata: WorktreeRecoveryState;
  profile: AgentProfile;
}

export class IntegrationService {
  private readonly worktrees = new WorktreeIsolation({
    credentialPath: isCredentialPath,
  });
  private readonly warn: (message: string) => void;
  private readonly profiles: () => HarnessConfig;

  constructor(
    warn: (message: string) => void,
    profiles: () => HarnessConfig,
  ) {
    this.warn = warn;
    this.profiles = profiles;
  }

  prepare(
    agentId: string,
    cwd: string,
    policy: AgentIsolationPolicy,
    signal?: AbortSignal,
  ) {
    return this.worktrees.prepare(agentId, cwd, policy, signal);
  }

  annotate(record: WorktreeRecord, recovery: WorktreeRecoveryState) {
    return this.worktrees.annotate(record, recovery);
  }

  capture(record: WorktreeRecord, signal?: AbortSignal) {
    return this.worktrees.capture(record, signal);
  }

  integrate(record: WorktreeRecord, signal?: AbortSignal) {
    return this.worktrees.integrate(record, signal);
  }

  keep(record: WorktreeRecord) {
    return this.worktrees.keep(record);
  }

  discard(record: WorktreeRecord) {
    return this.worktrees.discard(record);
  }

  prepareResolution(agentId: string, source: WorktreeRecord, signal?: AbortSignal) {
    return this.worktrees.prepareResolution(agentId, source, signal);
  }

  assertResolved(record: WorktreeRecord) {
    return this.worktrees.assertResolved(record);
  }

  resolvedBy(source: WorktreeRecord, resolverId: string) {
    return this.worktrees.resolvedBy(source, resolverId);
  }

  async recover(cwd: string): Promise<RecoveredSubagent[]> {
    const recovery = await this.worktrees.listRecoverable(cwd);
    for (const warning of recovery.warnings) this.warn(warning);
    const recovered: RecoveredSubagent[] = [];
    for (let record of recovery.records) {
      const metadata = record.recovery;
      if (!this.validWorktreeRecovery(metadata)) {
        this.warn(
          `Preserved worktree ${record.id}, but its recovery metadata is missing or invalid.`,
        );
        continue;
      }
      const profile = this.profiles().profiles[metadata.profile];
      if (!profile) {
        this.warn(
          `Preserved worktree ${record.id}, but subagent profile ${metadata.profile} is unavailable.`,
        );
        continue;
      }
      try {
        if (record.integrationStatus === "none") {
          const captured = await this.worktrees.capture(record);
          record = captured.record;
          if (!captured.hasChanges) {
            await this.worktrees.integrate(record);
            continue;
          }
        }
        this.validateWorktreePaths(record, profile, record.originWorkspace);
      } catch (error) {
        let warning =
          `Preserved worktree ${record.id}, but recovery validation failed: ${
            errorMessage(error)
          }`;
        try {
          await this.worktrees.keep(record);
        } catch (keepError) {
          warning += `; recording the keep decision also failed: ${
            errorMessage(keepError)
          }`;
        }
        this.warn(warning);
        continue;
      }
      recovered.push({ record, metadata, profile });
    }
    await this.worktrees.pruneTerminalArtifacts(cwd).catch((error) => {
      this.warn(
        `Unable to prune old terminal worktree artifacts: ${
          errorMessage(error)
        }`,
      );
    });
    return recovered;
  }

  validateWorktreePaths(
    record: WorktreeRecord,
    profile: AgentProfile,
    originCwd: string,
  ): void {
    for (const path of record.changedPaths) {
      const absolute = resolve(record.repoRoot, path);
      if (isCredentialPath(absolute)) {
        throw new Error(
          `Worktree result changes a credential-like path: ${path}`,
        );
      }
      let workspaceRelative: string;
      try {
        workspaceRelative = workspaceRelativePath(originCwd, absolute);
      } catch {
        throw new Error(`Worktree result changes outside the workspace: ${path}`);
      }
      const pathTools = ["edit", "write"].filter((tool) =>
        profile.tools.includes(tool),
      );
      if (
        pathTools.length > 0 &&
        pathTools.every(
          (tool) =>
            evaluateProfilePermission(profile, tool, workspaceRelative, originCwd) ===
            "deny",
        ) &&
        !profile.tools.includes("bash")
      ) {
        throw new Error(
          `Profile ${profile.description} denies the changed path: ${workspaceRelative}`,
        );
      }
    }
  }

  private validWorktreeRecovery(
    value: WorktreeRecoveryState | undefined,
  ): value is WorktreeRecoveryState {
    return (
      value !== undefined &&
      typeof value.profile === "string" &&
      typeof value.task === "string" &&
      typeof value.direct === "boolean" &&
      typeof value.planReadOnly === "boolean" &&
      typeof value.model === "string" &&
      typeof value.originSessionId === "string" &&
      (value.result === undefined || isJsonObject(value.result))
    );
  }
}
