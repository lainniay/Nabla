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

export const DEFAULT_GIT_TIMEOUT_MS = 30_000;
export const DEFAULT_LOCK_TIMEOUT_MS = 60_000;
export const DEFAULT_TERMINAL_RETENTION_MS = 30 * 24 * 60 * 60 * 1_000;
export const INTERNAL_GIT_IDENTITY = {
  GIT_AUTHOR_NAME: "Nabla",
  GIT_AUTHOR_EMAIL: "nabla@local",
  GIT_COMMITTER_NAME: "Nabla",
  GIT_COMMITTER_EMAIL: "nabla@local",
} as const;
