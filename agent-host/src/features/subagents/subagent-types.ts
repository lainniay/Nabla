import type { AgentSession } from "@earendil-works/pi-coding-agent";

import type { JsonObject } from "../../protocol/validation.ts";
import type {
  IntegrationStatus,
  IsolationBackend,
  PreparedIsolation,
  WorktreeRecord,
} from "./isolation/worktree.ts";

export interface SubagentOptions {
  task: string;
  profile: string;
  parentSignal?: AbortSignal;
  direct?: boolean;
  preparedIsolation?: PreparedIsolation;
  forceAutoIntegrate?: boolean;
  resolutionForAgentId?: string;
  discardWorktreeChanges?: boolean;
}

export interface ActiveSubagent {
  id: string;
  profile: string;
  task: string;
  direct: boolean;
  planReadOnly: boolean;
  lifecycle:
    | "queued"
    | "preparing_isolation"
    | "running"
    | "awaiting_integration"
    | "resolving";
  session?: AgentSession;
  originSession: AgentSession;
  originSessionId: string;
  controller: AbortController;
  startedAt: string;
  turns: number;
  maxTurns: number;
  model: string;
  isolationBackend: IsolationBackend;
  integrationStatus: IntegrationStatus;
  isolationWarning?: string;
  worktree?: WorktreeRecord;
}

export interface SubagentHandle {
  agent: ActiveSubagent;
  completion: Promise<JsonObject>;
}

export interface CompletedSubagent {
  agent: ActiveSubagent;
  result: JsonObject;
  record: WorktreeRecord;
}
