import type {
  ApprovalDecision,
  CapabilityMatcher,
  FileOperation,
  GrantBundle,
  GrantProposal,
  InvalidationKey,
} from "../../protocol/schemas/permissions.ts";

export type {
  ApprovalDecision,
  CapabilityMatcher,
  FileOperation,
  GrantBundle,
  GrantProposal,
  InvalidationKey,
} from "../../protocol/schemas/permissions.ts";

export type PolicyEffect = "allow" | "ask" | "deny";

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

export type FileCapabilityMatcher = Extract<
  CapabilityMatcher,
  { kind: "file" }
>;

export interface PermissionRule {
  id: string;
  effect: PolicyEffect;
  matcher: CapabilityMatcher;
  source: "builtin" | "managed" | "user" | "workspace" | "session";
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
