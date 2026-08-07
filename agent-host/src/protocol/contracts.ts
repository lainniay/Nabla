import type { ContextSnapshot } from "../features/context/model.ts";
import type { ApprovalDecision } from "../approval.ts";
import type {
  AgentConfigDiagnostic,
  AgentProfile,
} from "../features/subagents/profile-model.ts";
import type {
  ResourceSnapshot,
} from "../features/workspace/config.ts";
import type { GrantProposal } from "../features/permissions/model.ts";
import type { PlanArtifact } from "../features/plans/model.ts";
import type { PlanQuestion } from "../questions.ts";
import type {
  IsolationBackend,
  IntegrationStatus,
} from "../features/subagents/isolation/worktree.ts";
import {
  isJsonObject,
  requireArray,
  requireBoolean,
  requireFiniteNumber,
  requireObject,
  requireString,
  requireStringArray,
  type JsonObject,
} from "./validation.ts";

export interface HostPlanModeSnapshot {
  active: boolean;
  activeTools: string[];
}

export interface SandboxStatus {
  mode: "enforced" | "degraded" | "disabled";
  backend: "bubblewrap" | "seatbelt" | "none";
  filesystem: "workspace-write" | "full-access";
  network: "blocked" | "allowed";
  reason?: string;
}

export interface AgentProfileSnapshot {
  name: string;
  description: string;
  source: string;
  model: string | null;
  thinkingLevel: AgentProfile["thinkingLevel"] | null;
  skills: string[];
  tools: string[];
  permission: string;
  maxParallel: number;
  maxTurns: number;
  isolation: AgentProfile["isolation"];
  disabled: boolean;
  unavailableReason: string | null;
}

export interface ActiveAgentSnapshot {
  id: string;
  profile: string;
  task: string;
  lifecycle: string;
  startedAt: string;
  turns: number;
  maxTurns: number;
  model: string;
  originSessionId: string;
  isolationBackend: IsolationBackend;
  integrationStatus: IntegrationStatus;
  isolationWarning: string | null;
}

export interface WorktreeIntegrationSnapshot {
  backend: IsolationBackend;
  status: IntegrationStatus;
  warning: string | null;
  artifactId: string | null;
  changedPaths: string[];
  patchBytes: number;
  excludedPaths: string[];
  resolverAvailable: boolean;
}

export interface AgentsSnapshot {
  scopeId: string;
  revision: number;
  maxParallel: number;
  profiles: AgentProfileSnapshot[];
  active: ActiveAgentSnapshot[];
  pending: ActiveAgentSnapshot[];
  diagnostics: AgentConfigDiagnostic[];
}

export interface PendingIntegrationSnapshot {
  agent: ActiveAgentSnapshot;
  integration: WorktreeIntegrationSnapshot;
}

export interface BootstrapState {
  scopeId: string;
  planMode: HostPlanModeSnapshot;
  sandbox: SandboxStatus;
  plan: { artifact: PlanArtifact | null };
  resources: ResourceSnapshot;
  agents: AgentsSnapshot;
  context: ContextSnapshot;
  pendingIntegrations: PendingIntegrationSnapshot[];
  warnings: string[];
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

export function parseBootstrapState(value: unknown): BootstrapState {
  if (!isJsonObject(value)) throw new Error("bootstrap must be an object");
  requireString(value, "scopeId", "bootstrap");
  const planMode = requireObject(value, "planMode", "bootstrap");
  requireBoolean(planMode, "active", "bootstrap.planMode");
  requireStringArray(planMode, "activeTools", "bootstrap.planMode");

  const plan = requireObject(value, "plan", "bootstrap");
  if (plan.artifact !== null) validatePlanArtifact(plan.artifact);
  validateResources(requireObject(value, "resources", "bootstrap"));
  validateAgents(requireObject(value, "agents", "bootstrap"));
  validateContext(requireObject(value, "context", "bootstrap"));

  for (const [index, entry] of requireArray(
    value,
    "pendingIntegrations",
    "bootstrap",
  ).entries()) {
    if (!isJsonObject(entry)) {
      throw new Error(`bootstrap.pendingIntegrations[${index}] must be an object`);
    }
    validateAgent(
      requireObject(entry, "agent", `bootstrap.pendingIntegrations[${index}]`),
      `bootstrap.pendingIntegrations[${index}].agent`,
    );
    validateIntegration(
      requireObject(
        entry,
        "integration",
        `bootstrap.pendingIntegrations[${index}]`,
      ),
      `bootstrap.pendingIntegrations[${index}].integration`,
    );
  }
  requireStringArray(value, "warnings", "bootstrap");
  const sandboxValue: unknown = value.sandbox;
  const sandbox: SandboxStatus =
    sandboxValue === undefined
      ? {
          mode: "disabled",
          backend: "none",
          filesystem: "full-access",
          network: "allowed",
        }
      : (() => {
          if (!isJsonObject(sandboxValue)) {
            throw new Error("bootstrap.sandbox must be an object");
          }
          return {
            mode: requireString(
              sandboxValue,
              "mode",
              "bootstrap.sandbox",
            ) as SandboxStatus["mode"],
            backend: requireString(
              sandboxValue,
              "backend",
              "bootstrap.sandbox",
            ) as SandboxStatus["backend"],
            filesystem: requireString(
              sandboxValue,
              "filesystem",
              "bootstrap.sandbox",
            ) as SandboxStatus["filesystem"],
            network: requireString(
              sandboxValue,
              "network",
              "bootstrap.sandbox",
            ) as SandboxStatus["network"],
            ...(typeof sandboxValue.reason === "string"
              ? { reason: sandboxValue.reason }
              : {}),
          };
        })();
  return { ...(value as unknown as BootstrapState), sandbox };
}

function validatePlanArtifact(value: unknown): void {
  if (!isJsonObject(value)) throw new Error("bootstrap.plan.artifact must be an object or null");
  for (const field of [
    "id",
    "title",
    "summary",
    "bodyMarkdown",
    "handoffMarkdown",
    "sourceSessionId",
    "createdAt",
    "updatedAt",
  ]) {
    requireString(value, field, "bootstrap.plan.artifact");
  }
  requireFiniteNumber(value, "revision", "bootstrap.plan.artifact");
  requireStringArray(value, "assumptions", "bootstrap.plan.artifact");
  requireStringArray(value, "testPlan", "bootstrap.plan.artifact");
}

function validateResources(value: JsonObject): void {
  requireString(value, "scopeId", "bootstrap.resources");
  requireBoolean(value, "trusted", "bootstrap.resources");
  for (const field of [
    "contextFiles",
    "skills",
    "prompts",
    "extensions",
    "commands",
    "diagnostics",
  ]) {
    requireArray(value, field, "bootstrap.resources");
  }
  requireFiniteNumber(value, "revision", "bootstrap.resources");
}

function validateAgents(value: JsonObject): void {
  requireString(value, "scopeId", "bootstrap.agents");
  requireFiniteNumber(value, "revision", "bootstrap.agents");
  requireFiniteNumber(value, "maxParallel", "bootstrap.agents");
  requireArray(value, "profiles", "bootstrap.agents");
  for (const field of ["active", "pending"] as const) {
    for (const [index, agent] of requireArray(
      value,
      field,
      "bootstrap.agents",
    ).entries()) {
      if (!isJsonObject(agent)) {
        throw new Error(`bootstrap.agents.${field}[${index}] must be an object`);
      }
      validateAgent(agent, `bootstrap.agents.${field}[${index}]`);
    }
  }
  requireArray(value, "diagnostics", "bootstrap.agents");
}

function validateAgent(value: JsonObject, context: string): void {
  for (const field of [
    "id",
    "profile",
    "task",
    "lifecycle",
    "startedAt",
    "model",
    "originSessionId",
    "isolationBackend",
    "integrationStatus",
  ]) {
    requireString(value, field, context);
  }
  requireFiniteNumber(value, "turns", context);
  requireFiniteNumber(value, "maxTurns", context);
}

function validateIntegration(value: JsonObject, context: string): void {
  requireString(value, "backend", context);
  requireString(value, "status", context);
  requireStringArray(value, "changedPaths", context);
  requireStringArray(value, "excludedPaths", context);
  requireFiniteNumber(value, "patchBytes", context);
  requireBoolean(value, "resolverAvailable", context);
}

function validateContext(value: JsonObject): void {
  requireString(value, "scopeId", "bootstrap.context");
  requireFiniteNumber(value, "revision", "bootstrap.context");
  requireString(value, "usageState", "bootstrap.context");
  for (const field of [
    "estimatedUnfilteredTokens",
    "estimatedNextRequestTokens",
    "estimatedPrunedThisRequestTokens",
    "estimatedCurrentlyPrunableTokens",
    "estimatedCumulativeAvoidedTokens",
    "compactionCount",
    "epoch",
  ]) {
    requireFiniteNumber(value, field, "bootstrap.context");
  }
  for (const field of [
    "categories",
    "pruning",
    "topConsumers",
    "recentCompactions",
  ]) {
    requireArray(value, field, "bootstrap.context");
  }
  requireObject(value, "policy", "bootstrap.context");
}
