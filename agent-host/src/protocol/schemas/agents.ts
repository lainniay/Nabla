import { Type, type Static } from "typebox";

const ThinkingLevelSchema = Type.Union([
  Type.Literal("off"),
  Type.Literal("minimal"),
  Type.Literal("low"),
  Type.Literal("medium"),
  Type.Literal("high"),
  Type.Literal("xhigh"),
  Type.Literal("max"),
]);

const IsolationModeSchema = Type.Union([
  Type.Literal("none"),
  Type.Literal("auto"),
  Type.Literal("worktree"),
]);

const IntegrationModeSchema = Type.Union([
  Type.Literal("source"),
  Type.Literal("auto"),
  Type.Literal("ask"),
  Type.Literal("manual"),
]);

export const IsolationBackendSchema = Type.Union([
  Type.Literal("shared"),
  Type.Literal("shared_fallback"),
  Type.Literal("worktree"),
]);

export type IsolationBackend = Static<typeof IsolationBackendSchema>;

export const IntegrationStatusSchema = Type.Union([
  Type.Literal("none"),
  Type.Literal("pending"),
  Type.Literal("applying"),
  Type.Literal("applied"),
  Type.Literal("kept"),
  Type.Literal("conflicted"),
  Type.Literal("needs_reconciliation"),
  Type.Literal("discarded"),
]);

export type IntegrationStatus = Static<typeof IntegrationStatusSchema>;

export const AgentConfigDiagnosticSchema = Type.Object({
  type: Type.Union([Type.Literal("warning"), Type.Literal("error")]),
  message: Type.String(),
  path: Type.Optional(Type.String()),
  profile: Type.Optional(Type.String()),
});

export type AgentConfigDiagnostic = Static<typeof AgentConfigDiagnosticSchema>;

export const AgentProfileSnapshotSchema = Type.Object({
  name: Type.String(),
  description: Type.String(),
  source: Type.String(),
  model: Type.Union([Type.Null(), Type.String()]),
  thinkingLevel: Type.Union([Type.Null(), ThinkingLevelSchema]),
  skills: Type.Array(Type.String()),
  tools: Type.Array(Type.String()),
  permission: Type.String(),
  maxParallel: Type.Number(),
  maxTurns: Type.Number(),
  isolation: Type.Object({
    mode: IsolationModeSchema,
    integration: IntegrationModeSchema,
  }),
  disabled: Type.Boolean(),
  unavailableReason: Type.Union([Type.Null(), Type.String()]),
});

export type AgentProfileSnapshot = Static<typeof AgentProfileSnapshotSchema>;

export const ActiveAgentSnapshotSchema = Type.Object({
  id: Type.String(),
  profile: Type.String(),
  task: Type.String(),
  lifecycle: Type.String(),
  startedAt: Type.String(),
  turns: Type.Number(),
  maxTurns: Type.Number(),
  model: Type.String(),
  originSessionId: Type.String(),
  isolationBackend: IsolationBackendSchema,
  integrationStatus: IntegrationStatusSchema,
  isolationWarning: Type.Union([Type.Null(), Type.String()]),
});

export type ActiveAgentSnapshot = Static<typeof ActiveAgentSnapshotSchema>;

export const WorktreeIntegrationSnapshotSchema = Type.Object({
  backend: IsolationBackendSchema,
  status: IntegrationStatusSchema,
  warning: Type.Union([Type.Null(), Type.String()]),
  artifactId: Type.Union([Type.Null(), Type.String()]),
  changedPaths: Type.Array(Type.String()),
  patchBytes: Type.Number(),
  excludedPaths: Type.Array(Type.String()),
  resolverAvailable: Type.Boolean(),
});

export type WorktreeIntegrationSnapshot = Static<
  typeof WorktreeIntegrationSnapshotSchema
>;

export const AgentsSnapshotSchema = Type.Object({
  scopeId: Type.String(),
  revision: Type.Number(),
  maxParallel: Type.Number(),
  profiles: Type.Array(AgentProfileSnapshotSchema),
  active: Type.Array(ActiveAgentSnapshotSchema),
  pending: Type.Array(ActiveAgentSnapshotSchema),
  diagnostics: Type.Array(AgentConfigDiagnosticSchema),
});

export type AgentsSnapshot = Static<typeof AgentsSnapshotSchema>;

export const PendingIntegrationSnapshotSchema = Type.Object({
  agent: ActiveAgentSnapshotSchema,
  integration: WorktreeIntegrationSnapshotSchema,
});

export type PendingIntegrationSnapshot = Static<
  typeof PendingIntegrationSnapshotSchema
>;
