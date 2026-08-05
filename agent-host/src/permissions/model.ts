export type PolicyEffect = "allow" | "ask" | "deny";

export type ApprovalDecision =
  | "allow_once"
  | "allow_session"
  | "allow_workspace"
  | "deny";

export type FileOperation =
  | "read"
  | "list"
  | "create"
  | "write"
  | "append"
  | "rename"
  | "delete";

export interface ExecCapability {
  kind: "exec";
  executable: string;
  argv: string[];
  cwd: string;
  environment: Record<string, string>;
}

export interface FileCapability {
  kind: "file";
  operation: FileOperation;
  path: string;
  destination?: string;
}

export interface NetworkCapability {
  kind: "network";
  operation: "connect" | "listen";
  host: string;
  port?: number;
  protocol?: string;
}

export interface OpaqueCodeCapability {
  kind: "opaque_code";
  runtime: string;
  digest: string;
  reason: string;
}

export type CapabilityAtom =
  | ExecCapability
  | FileCapability
  | NetworkCapability
  | OpaqueCodeCapability;

export interface PermissionIntent {
  id: string;
  toolCallId: string;
  sessionId: string;
  workspaceId: string;
  tool: string;
  normalizedInput: unknown;
  atoms: CapabilityAtom[];
  digest: string;
}

export interface ExecCapabilityMatcher {
  kind: "exec";
  executable: string;
  argv?: string[];
  cwd?: string;
  environment?: Record<string, string>;
  manifestDigest?: string;
}

export interface FileCapabilityMatcher {
  kind: "file";
  operation: FileOperation;
  path: string;
  recursive?: boolean;
  destination?: string;
}

export interface NetworkCapabilityMatcher {
  kind: "network";
  operation: "connect" | "listen";
  host: string;
  port?: number;
  protocol?: string;
}

export interface OpaqueCodeCapabilityMatcher {
  kind: "opaque_code";
  runtime: string;
  digest: string;
}

export interface ToolCapabilityMatcher {
  kind: "tool";
  tool: string;
  inputDigest?: string;
}

export interface ShellIntentMatcher {
  kind: "shell_intent";
  command: string;
}

export interface OpaqueShellExactMatcher {
  kind: "opaque_shell_exact";
  command: string;
}

export type CapabilityMatcher =
  | ExecCapabilityMatcher
  | FileCapabilityMatcher
  | NetworkCapabilityMatcher
  | OpaqueCodeCapabilityMatcher
  | ToolCapabilityMatcher
  | ShellIntentMatcher
  | OpaqueShellExactMatcher;

export interface PermissionRule {
  id: string;
  effect: PolicyEffect;
  matcher: CapabilityMatcher;
  source: "builtin" | "managed" | "user" | "workspace" | "session";
}

export interface InvalidationKey {
  kind: "file_digest" | "workspace_generation" | "git_common_directory";
  path?: string;
  value: string;
}

export interface GrantBundle {
  scope: "once" | "session" | "workspace";
  workspaceId: string;
  sessionId?: string;
  matchers: CapabilityMatcher[];
  invalidationKeys?: InvalidationKey[];
}

export interface PermissionExplanation {
  summary: string;
  details: string[];
  risk: "normal" | "elevated" | "high";
}

export interface ToolContext {
  requestId: string;
  toolCallId: string;
  sessionId: string;
  workspaceId: string;
  cwd: string;
  environment?: Record<string, string>;
}

export interface PermissionAdapter<TInput> {
  normalize(context: ToolContext, input: TInput): PermissionIntent;
  proposeGrants(intent: PermissionIntent): GrantBundle[];
  explain(intent: PermissionIntent): PermissionExplanation;
}

export interface CapabilityGrantSet {
  matchers: CapabilityMatcher[];
}

export interface FileSystemProfile {
  read: string[];
  write: string[];
}

export interface NetworkProfile {
  allow: Array<{ host: string; port?: number }>;
}

export interface EnvironmentProfile {
  inherit: string[];
  set: Record<string, string>;
}

export interface ExecutionProfile {
  filesystem: FileSystemProfile;
  network: NetworkProfile;
  environment: EnvironmentProfile;
  backend: "none" | "native" | "container";
}
