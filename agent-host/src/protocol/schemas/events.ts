import type { JsonObject } from "../validation.ts";
import type { ContextSnapshot } from "./context.ts";
import type { PlanArtifact } from "./plans.ts";
import type { PlanQuestion } from "./questions.ts";
import type {
  ActiveAgentSnapshot,
  AgentsSnapshot,
  IntegrationStatus,
  WorktreeIntegrationSnapshot,
} from "./agents.ts";
import type { GrantProposal, ApprovalDecision } from "./permissions.ts";
import type { ResourceSnapshot } from "./workspace.ts";

export interface HostPlanModeSnapshot {
  active: boolean;
  activeTools: string[];
}

export type SubagentStateEvent =
  | "queued"
  | "preparing_isolation"
  | "isolated"
  | "shared"
  | "shared_fallback"
  | "started"
  | "completed"
  | "awaiting_integration"
  | "limit_reached"
  | "failed"
  | "cancelled"
  | "resolving";

export type HostEvent =
  | { type: "question_request"; requestId: string; questions: PlanQuestion[] }
  | { type: "question_cancelled"; requestId: string }
  | { type: "plan_ready"; artifact: PlanArtifact }
  | { type: "plan_state"; artifact: PlanArtifact | null }
  | {
      type: "turn_timing";
      phase: "started";
      turnId: string;
      startedAt: string;
    }
  | {
      type: "turn_timing";
      phase: "completed";
      turnId: string;
      startedAt: string;
      endedAt: string;
      durationMs: number;
    }
  | { type: "context_budget"; snapshot: ContextSnapshot; policyWarning?: string }
  | { type: "host_warning"; message: string }
  | {
      type: "workspace_state";
      scopeId: string;
      resources: ResourceSnapshot;
      agents: AgentsSnapshot;
    }
  | { type: "agents_state"; snapshot: AgentsSnapshot }
  | {
      type: "subagent_integration";
      event: IntegrationStatus | "resolving" | "pending";
      agent: ActiveAgentSnapshot;
      integration: WorktreeIntegrationSnapshot;
      error?: string;
      resolvedBy?: string;
    }
  | {
      type: "subagent_state";
      event: SubagentStateEvent;
      agent: ActiveAgentSnapshot;
      warning?: string;
      result?: JsonObject;
      error?: string;
    }
  | {
      type: "auth_complete";
      flowId: string;
      providerId: string;
      credentialType: string;
      selectedModel: unknown;
    }
  | {
      type: "auth_prompt";
      flowId: string;
      promptId: string;
      promptType: string;
      message: string;
      placeholder?: string;
      options?: unknown;
    }
  | { type: "auth_prompt_cancelled"; flowId: string; promptId: string }
  | { type: "auth_notify"; flowId: string; event: unknown }
  | { type: "plan_mode_state"; active: boolean; activeTools: string[] }
  | {
      type: "session_list_progress";
      browserId: string;
      scope: string;
      loaded: number;
      total: number;
    }
  | {
      type: "approval_request";
      requestId: string;
      toolCallId: string;
      sessionId: string;
      workspaceId: string;
      summary: string;
      risk: "normal" | "elevated" | "high" | "credential" | "outside_workspace";
      intentDigest: string;
      availableDecisions: ApprovalDecision[];
      sessionGrant?: GrantProposal;
      workspaceGrant?: GrantProposal;
      toolName: string;
      input: unknown;
      agentId?: string;
      agentProfile?: string;
      model?: string;
      reason?: string;
    }
  | {
      type: "response";
      id?: string;
      command: string;
      success: boolean;
      data?: unknown;
      error?: string;
    }
  | { type: "host_protocol_error"; error: string };
