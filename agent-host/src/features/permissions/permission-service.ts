import { realpathSync } from "node:fs";
import { resolve } from "node:path";

import type {
  ToolCallEvent,
  ToolCallEventResult,
} from "@earendil-works/pi-coding-agent";

import type { ApprovalDecision } from "../../approval.ts";
import { isCredentialPath } from "./filesystem/credential.ts";
import type { AgentProfile } from "../subagents/profile-model.ts";
import { workspacePathError } from "./filesystem/path.ts";
import { resolveWorkspaceIdentity } from "./workspace-identity.ts";
import { JsonlPermissionAuditLog } from "./audit-log.ts";
import { ApprovalBroker as PermissionApprovalBroker } from "./approvals/broker.ts";
import { PermissionKernel } from "./kernel.ts";
import type { Authorization } from "./kernel.ts";
import { buildSandboxProfile } from "./execution/sandbox-profile.ts";
import type {
  ExecutionPermit,
  SandboxExecutionProfile,
} from "./execution/sandbox-profile.ts";
import type { SandboxCapability } from "./execution/sandbox-capability.ts";
import type {
  ExecutionProfile,
  PermissionIntent,
  PermissionRule,
  ToolContext,
} from "./model.ts";
import { mutatesManagedWorktree } from "./managed-worktree.ts";
import { PolicyStore } from "./policy-store.ts";
import { ShellAdapter } from "./adapters/shell.ts";
import type { WorkspaceGrantSnapshot } from "./approvals/workspace-store.ts";
import type { InteractionBroker } from "../interactions/interaction-broker.ts";
import { permissionIntentForTool } from "./tool-intent.ts";
import { compileAgentProfileRules } from "./policy/profile-compiler.ts";
import { PLAN_MODE_POLICY } from "../plans/plan-controller.ts";
import type { JsonObject } from "../../protocol/validation.ts";

const EXTERNAL_TOOL_EXECUTION_PROFILE: ExecutionProfile = {
  backend: "none",
  filesystem: { read: ["*"], write: ["*"] },
  network: { allow: [{ host: "*" }] },
  environment: { inherit: [], set: {} },
};

const DEGRADED_CAPABILITY: SandboxCapability = {
  mode: "degraded",
  backend: "none",
  supportsFilesystemIsolation: false,
  supportsNetworkIsolation: false,
};

export interface ToolAuthorizationContext {
  cwd: string;
  signal?: AbortSignal;
  agent?: {
    agentId?: string;
    profile?: string;
    model?: string;
    profileConfig?: AgentProfile;
    planReadOnly?: boolean;
    sessionId?: string;
  };
}

export interface AuthorizeBashInput {
  toolCallId: string;
  command: string;
  timeout?: number;
  cwd: string;
  signal?: AbortSignal;
  agent?: ToolAuthorizationContext["agent"];
}

export interface BashAuthorization extends ExecutionPermit {
  decision: "allow" | "deny";
  reason?: string;
}

export class PermissionService {
  private readonly policies = new PolicyStore();
  private readonly approvals = new PermissionApprovalBroker();
  private readonly kernel = new PermissionKernel(
    this.policies,
    this.approvals,
    new JsonlPermissionAuditLog(),
  );
  private readonly pending = new Map<string, Authorization>();
  private readonly pendingBash = new Map<
    string,
    {
      authorization: Authorization;
      intent: PermissionIntent;
      profile: SandboxExecutionProfile;
    }
  >();
  private readonly shellAdapter = new ShellAdapter();
  private readonly sessionIdProvider: () => string;
  private readonly cwdProvider: () => string;
  private readonly sandboxCapability: () => SandboxCapability;
  private readonly interactions: InteractionBroker;
  private readonly send: (event: JsonObject) => void;
  private readonly planMode: { current(): boolean };
  private readonly isConnected: () => boolean;

  constructor(
    interactions: InteractionBroker,
    send: (event: JsonObject) => void,
    planMode: { current(): boolean },
    isConnected: () => boolean,
    scope: { sessionId(): string; cwd(): string },
    sandbox?: { capability(): SandboxCapability },
  ) {
    this.interactions = interactions;
    this.send = send;
    this.planMode = planMode;
    this.isConnected = isConnected;
    this.sessionIdProvider = scope.sessionId;
    this.cwdProvider = scope.cwd;
    this.sandboxCapability = sandbox?.capability ?? (() => DEGRADED_CAPABILITY);
    this.policies.setBuiltin(
      ["ask_user", "submit_plan"].map(
        (tool): PermissionRule => ({
          id: `builtin-tool-${tool}`,
          effect: "allow",
          source: "builtin",
          matcher: { kind: "tool", tool },
        }),
      ),
    );
  }

  async authorizeTool(
    event: ToolCallEvent,
    context: ToolAuthorizationContext,
  ): Promise<ToolCallEventResult | undefined> {
    const core = await this.authorizeCore(event, context);
    if ("blocked" in core) {
      return { block: true, reason: core.reason };
    }
    if (
      !this.kernel.consumeForExecution(
        core.authorization,
        core.intent,
        EXTERNAL_TOOL_EXECUTION_PROFILE,
      )
    ) {
      return { block: true, reason: "Tool input changed after approval" };
    }
    this.pending.set(event.toolCallId, core.authorization);
    return undefined;
  }

  async authorizeBash(input: AuthorizeBashInput): Promise<BashAuthorization> {
    const event: ToolCallEvent = {
      type: "tool_call",
      toolCallId: input.toolCallId,
      toolName: "bash",
      input: {
        command: input.command,
        ...(input.timeout === undefined ? {} : { timeout: input.timeout }),
      },
    };
    const core = await this.authorizeCore(event, {
      cwd: input.cwd,
      signal: input.signal,
      agent: input.agent,
    });
    if ("blocked" in core) {
      return {
        id: `denied-${input.toolCallId}`,
        decision: "deny",
        reason: core.reason,
        intentDigest: "",
        sandboxProfile: buildSandboxProfile(
          emptyIntent(input),
          input.cwd,
          this.sandboxCapability(),
        ),
      };
    }
    const profile = buildSandboxProfile(
      core.intent,
      input.cwd,
      this.sandboxCapability(),
    );
    if (
      !this.kernel.consumeForExecution(
        core.authorization,
        core.intent,
        toExecutionProfile(profile),
      )
    ) {
      return {
        id: core.authorization.requestId,
        decision: "deny",
        reason: "Tool input changed after approval",
        intentDigest: core.intent.digest,
        sandboxProfile: profile,
      };
    }
    this.pendingBash.set(input.toolCallId, {
      authorization: core.authorization,
      intent: core.intent,
      profile,
    });
    return {
      id: core.authorization.requestId,
      decision: "allow",
      intentDigest: core.intent.digest,
      sandboxProfile: profile,
    };
  }

  finishBash(toolCallId: string, succeeded: boolean): void {
    const entry = this.pendingBash.get(toolCallId);
    if (!entry) return;
    this.pendingBash.delete(toolCallId);
    this.kernel.recordExecutionResult(
      entry.authorization,
      toExecutionProfile(entry.profile),
      succeeded,
    );
  }

  private async authorizeCore(
    event: ToolCallEvent,
    context: ToolAuthorizationContext,
  ): Promise<
    | { authorization: Authorization; intent: PermissionIntent }
    | { blocked: true; reason: string }
  > {
    const toolName = event.toolName;
    const input = event.input as Record<string, unknown>;
    const path = typeof input.path === "string" ? input.path : undefined;
    const agent = context.agent ?? {};
    const profile = agent.profileConfig;
    if (profile && !profile.tools.includes(toolName)) {
      return {
        blocked: true,
        reason: `Tool ${toolName} is not exposed to profile ${agent.profile}`,
      };
    }
    const sessionId = agent.sessionId ?? this.tryCurrentScopeId();
    if (!sessionId) {
      return { blocked: true, reason: "Permission scope is unavailable" };
    }
    const identity = resolveWorkspaceIdentity(context.cwd);
    const permissionContext: ToolContext = {
      requestId: `request-${event.toolCallId}`,
      toolCallId: event.toolCallId,
      sessionId,
      workspaceId: identity.id,
      cwd: context.cwd,
    };
    const normalize = () =>
      permissionIntentForTool(
        permissionContext,
        toolName,
        event.input,
        this.shellAdapter,
      );
    const intent = normalize();
    const shellAnalysis =
      intent.tool === "bash" ? this.shellAdapter.analysis(intent) : undefined;
    const additionalRules: PermissionRule[] = [];
    const addToolRule = (
      id: string,
      effect: "ask" | "deny",
      source: PermissionRule["source"],
    ) => {
      additionalRules.push({
        id,
        effect,
        source,
        matcher: { kind: "tool", tool: toolName },
      });
    };
    if (profile) {
      additionalRules.push(...compileAgentProfileRules(profile, context.cwd));
    }
    if (agent.agentId && mutatesManagedWorktree(intent)) {
      addToolRule("managed-worktree-boundary", "deny", "managed");
    }
    if (agent.planReadOnly || (!agent.agentId && this.planMode.current())) {
      additionalRules.push(...PLAN_MODE_POLICY.permissionRules);
    }
    const workspaceRoot = realpathSync(context.cwd);
    additionalRules.push(
      {
        id: "builtin-workspace-read",
        effect: "allow",
        source: "builtin",
        matcher: {
          kind: "file",
          operation: "read",
          path: workspaceRoot,
          recursive: true,
        },
      },
      {
        id: "builtin-workspace-list",
        effect: "allow",
        source: "builtin",
        matcher: {
          kind: "file",
          operation: "list",
          path: workspaceRoot,
          recursive: true,
        },
      },
    );
    for (const atom of intent.atoms) {
      if (
        atom.kind === "file" &&
        (atom.operation === "read" || atom.operation === "list") &&
        isCredentialPath(atom.path)
      ) {
        additionalRules.push({
          id: `builtin-credential-deny-${atom.operation}-${atom.path}`,
          effect: "deny",
          source: "builtin",
          matcher: {
            kind: "file",
            operation: atom.operation,
            path: atom.path,
          },
        });
      }
    }
    if (
      shellAnalysis?.safety.readOnly &&
      intent.atoms.some((atom) => atom.kind === "exec")
    ) {
      for (const atom of intent.atoms) {
        if (atom.kind === "exec") {
          additionalRules.push({
            id: `builtin-readonly-bash-${additionalRules.length}`,
            effect: "allow",
            source: "builtin",
            matcher: {
              kind: "exec",
              executable: atom.executable,
              argv: atom.argv,
              cwd: atom.cwd,
            },
          });
        } else if (
          atom.kind === "file" &&
          atom.path === "/dev/null"
        ) {
          additionalRules.push({
            id: `builtin-readonly-bash-${additionalRules.length}`,
            effect: "allow",
            source: "builtin",
            matcher: {
              kind: "file",
              operation: atom.operation,
              path: atom.path,
            },
          });
        }
      }
    }
    if (
      this.sandboxCapability().mode === "enforced" &&
      intent.atoms.some((atom) => atom.kind === "exec")
    ) {
      const sandboxAutoAllow =
        shellAnalysis !== undefined &&
        !shellAnalysis.safety.opaque &&
        !shellAnalysis.safety.network &&
        !shellAnalysis.safety.destructive;
      if (sandboxAutoAllow) {
        for (const atom of intent.atoms) {
          if (atom.kind === "exec") {
            additionalRules.push({
              id: `builtin-sandbox-bash-${additionalRules.length}`,
              effect: "allow",
              source: "builtin",
              matcher: {
                kind: "exec",
                executable: atom.executable,
                argv: atom.argv,
                cwd: atom.cwd,
              },
            });
          } else if (atom.kind === "file") {
            additionalRules.push({
              id: `builtin-sandbox-bash-${additionalRules.length}`,
              effect: "allow",
              source: "builtin",
              matcher: {
                kind: "file",
                operation: atom.operation,
                path: atom.path,
                ...(atom.destination === undefined
                  ? {}
                  : { destination: atom.destination }),
              },
            });
          }
        }
      }
    }

    let risk: "normal" | "high" | "credential" | "outside_workspace" =
      intent.atoms.some((atom) => atom.kind === "opaque_code")
        ? "high"
        : "normal";
    let reason =
      risk === "high"
        ? "The request contains code that cannot be statically decomposed"
        : "Permission is required for every capability in this request";
    if (path) {
      const pathError = await workspacePathError(context.cwd, path);
      if (isCredentialPath(resolve(context.cwd, path))) {
        reason = "Path may contain credentials";
        risk = "credential";
      } else if (pathError) {
        reason = pathError;
        risk = "outside_workspace";
      }
    }

    const authorization = await this.kernel.authorize(
      permissionContext.requestId,
      intent,
      identity,
      async ({ intent: requestedIntent, proposals }, approvalSignal) => {
        if (!this.isConnected()) return "deny";
        const sessionGrant = proposals.find(
          (proposal) => proposal.scope === "session",
        );
        const workspaceGrant = proposals.find(
          (proposal) => proposal.scope === "workspace",
        );
        const availableDecisions: ApprovalDecision[] = ["allow_once"];
        if (sessionGrant) availableDecisions.push("allow_session");
        if (workspaceGrant) availableDecisions.push("allow_workspace");
        availableDecisions.push("deny");
        return this.interactions.requestApproval(
          {
            requestId: permissionContext.requestId,
            toolCallId: event.toolCallId,
            sessionId: permissionContext.sessionId,
            workspaceId: permissionContext.workspaceId,
            summary: reason,
            risk,
            intentDigest: requestedIntent.digest,
            availableDecisions,
            ...(sessionGrant ? { sessionGrant } : {}),
            ...(workspaceGrant ? { workspaceGrant } : {}),
            toolName,
            input: event.input,
            agentId: agent.agentId,
            agentProfile: agent.profile,
            model: agent.model,
            reason,
          },
          approvalSignal,
          (approvalEvent) => this.send(approvalEvent),
        );
      },
      context.signal,
      additionalRules,
      !agent.agentId,
      risk,
    );
    if (
      authorization.evaluation.effect === "deny" ||
      authorization.decision === "deny"
    ) {
      return {
        blocked: true,
        reason:
          authorization.evaluation.effect === "deny"
            ? "Denied by permission policy"
            : "Denied by user",
      };
    }
    return { authorization, intent: normalize() };
  }

  finishTool(toolCallId: string, succeeded: boolean): void {
    const authorization = this.pending.get(toolCallId);
    if (!authorization) return;
    this.pending.delete(toolCallId);
    this.kernel.recordExecutionResult(
      authorization,
      EXTERNAL_TOOL_EXECUTION_PROFILE,
      succeeded,
    );
  }

  workspaceRules(): WorkspaceGrantSnapshot {
    return this.approvals.workspace.snapshot(this.identity());
  }

  revokeWorkspaceRule(ruleId: string): WorkspaceGrantSnapshot {
    return this.approvals.workspace.revoke(this.identity(), ruleId);
  }

  clearWorkspaceRules(): WorkspaceGrantSnapshot {
    return this.approvals.workspace.clear(this.identity());
  }

  private tryCurrentScopeId(): string | undefined {
    try {
      return this.currentScopeId();
    } catch {
      return undefined;
    }
  }

  private currentScopeId(): string {
    return this.sessionIdProvider();
  }

  private identity() {
    return resolveWorkspaceIdentity(this.cwdProvider());
  }

}

function toExecutionProfile(
  profile: SandboxExecutionProfile,
): ExecutionProfile {
  // Audit-only profile; the Rust sandbox owns real execution.
  return {
    backend: "none",
    filesystem: { read: ["*"], write: profile.filesystem.readWrite },
    network: { allow: profile.network === "allowed" ? [{ host: "*" }] : [] },
    environment: { inherit: [], set: {} },
  };
}

function emptyIntent(input: AuthorizeBashInput): PermissionIntent {
  return {
    id: `denied-${input.toolCallId}`,
    toolCallId: input.toolCallId,
    sessionId: "",
    workspaceId: "",
    tool: "bash",
    normalizedInput: { command: input.command },
    atoms: [],
    digest: "",
  };
}
