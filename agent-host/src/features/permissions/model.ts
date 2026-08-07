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
  | "truncate"
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
  operation: FileOperation | "*";
  path: string;
  /** Treat path as a glob pattern instead of a containment prefix. */
  pattern?: boolean;
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

export interface ShellDigestMatcher {
  kind: "shell_digest";
  digest: string;
}

export interface ShellCommandMatcher {
  kind: "shell_command";
  pattern: string;
}

export type CapabilityMatcher =
  | ExecCapabilityMatcher
  | FileCapabilityMatcher
  | NetworkCapabilityMatcher
  | OpaqueCodeCapabilityMatcher
  | ToolCapabilityMatcher
  | ShellDigestMatcher
  | ShellCommandMatcher;

export interface PermissionRule {
  id: string;
  effect: PolicyEffect;
  matcher: CapabilityMatcher;
  source: "builtin" | "managed" | "user" | "workspace" | "session";
}

export interface InvalidationKey {
  kind:
    | "file_digest"
    | "npm_script_digest"
    | "workspace_generation"
    | "git_common_directory";
  path?: string;
  selector?: string;
  value: string;
}

export interface GrantBundle {
  scope: "once" | "session" | "workspace";
  workspaceId: string;
  sessionId?: string;
  matchers: CapabilityMatcher[];
  invalidationKeys?: InvalidationKey[];
}

export type GrantProposal = GrantBundle;

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
