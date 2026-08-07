import { Type, type Static } from "typebox";

export const ApprovalDecisionSchema = Type.Union([
  Type.Literal("allow_once"),
  Type.Literal("allow_session"),
  Type.Literal("allow_workspace"),
  Type.Literal("deny"),
]);

export type ApprovalDecision = Static<typeof ApprovalDecisionSchema>;

export const FileOperationSchema = Type.Union([
  Type.Literal("read"),
  Type.Literal("list"),
  Type.Literal("create"),
  Type.Literal("write"),
  Type.Literal("truncate"),
  Type.Literal("append"),
  Type.Literal("rename"),
  Type.Literal("delete"),
]);

export type FileOperation = Static<typeof FileOperationSchema>;

export const CapabilityMatcherSchema = Type.Union([
  Type.Object({
    kind: Type.Literal("exec"),
    executable: Type.String(),
    argv: Type.Optional(Type.Array(Type.String())),
    cwd: Type.Optional(Type.String()),
    environment: Type.Optional(Type.Record(Type.String(), Type.String())),
    manifestDigest: Type.Optional(Type.String()),
  }),
  Type.Object({
    kind: Type.Literal("file"),
    operation: Type.Union([FileOperationSchema, Type.Literal("*")]),
    path: Type.String(),
    pattern: Type.Optional(Type.Boolean()),
    recursive: Type.Optional(Type.Boolean()),
    destination: Type.Optional(Type.String()),
  }),
  Type.Object({
    kind: Type.Literal("network"),
    operation: Type.Union([Type.Literal("connect"), Type.Literal("listen")]),
    host: Type.String(),
    port: Type.Optional(Type.Number()),
    protocol: Type.Optional(Type.String()),
  }),
  Type.Object({
    kind: Type.Literal("opaque_code"),
    runtime: Type.String(),
    digest: Type.String(),
  }),
  Type.Object({
    kind: Type.Literal("tool"),
    tool: Type.String(),
    inputDigest: Type.Optional(Type.String()),
  }),
  Type.Object({
    kind: Type.Literal("shell_digest"),
    digest: Type.String(),
  }),
  Type.Object({
    kind: Type.Literal("shell_command"),
    pattern: Type.String(),
  }),
]);

export type CapabilityMatcher = Static<typeof CapabilityMatcherSchema>;

export const InvalidationKeySchema = Type.Object({
  kind: Type.Union([
    Type.Literal("file_digest"),
    Type.Literal("npm_script_digest"),
    Type.Literal("workspace_generation"),
    Type.Literal("git_common_directory"),
  ]),
  path: Type.Optional(Type.String()),
  selector: Type.Optional(Type.String()),
  value: Type.String(),
});

export const GrantProposalSchema = Type.Object({
  scope: Type.Union([
    Type.Literal("once"),
    Type.Literal("session"),
    Type.Literal("workspace"),
  ]),
  workspaceId: Type.String(),
  sessionId: Type.Optional(Type.String()),
  matchers: Type.Array(CapabilityMatcherSchema),
  invalidationKeys: Type.Optional(Type.Array(InvalidationKeySchema)),
});

export type GrantProposal = Static<typeof GrantProposalSchema>;
