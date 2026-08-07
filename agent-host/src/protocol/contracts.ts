// Protocol wire types live in protocol/schemas. This module is a stable
// re-export registry so application code keeps a single import surface.
export type {
  ActiveAgentSnapshot,
  AgentConfigDiagnostic,
  AgentProfileSnapshot,
  AgentsSnapshot,
  PendingIntegrationSnapshot,
  WorktreeIntegrationSnapshot,
} from "./schemas/agents.ts";
export type { IntegrationStatus, IsolationBackend } from "./schemas/agents.ts";
export { parseBootstrapState } from "./schemas/bootstrap.ts";
export type { BootstrapState } from "./schemas/bootstrap.ts";
export type { ContextSnapshot } from "./schemas/context.ts";
export type {
  HostEvent,
  HostPlanModeSnapshot,
  SubagentStateEvent,
} from "./schemas/events.ts";
export type { PlanArtifact } from "./schemas/plans.ts";
export type { ApprovalDecision, GrantProposal } from "./schemas/permissions.ts";
export type { PlanQuestion } from "./schemas/questions.ts";
export type { SandboxStatus } from "./schemas/sandbox.ts";
export type { ResourceSnapshot } from "./schemas/workspace.ts";
