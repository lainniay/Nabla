import { resolve } from "node:path";

import type {
  ToolCallEvent,
  ToolCallEventResult,
} from "@earendil-works/pi-coding-agent";

import type { ApprovalDecision } from "../../approval.ts";
import {
  agentPermissionEffect,
  isCredentialPath,
  type AgentProfile,
} from "../../harness.ts";
import { workspacePathError } from "../../workspace.ts";
import { resolveWorkspaceIdentity } from "../../permissions/workspace-identity.ts";
import { JsonlPermissionAuditLog } from "../../permissions/audit-log.ts";
import { ApprovalBroker as PermissionApprovalBroker } from "../../permissions/approvals/broker.ts";
import { PermissionKernel } from "../../permissions/kernel.ts";
import type { Authorization } from "../../permissions/kernel.ts";
import { ExecutionBroker } from "../../permissions/execution/broker.ts";
import { DirectRunner } from "../../permissions/execution/direct-runner.ts";
import type {
  ExecutionProfile,
  PermissionRule,
  ToolContext,
} from "../../permissions/model.ts";
import { mutatesManagedWorktree } from "../../permissions/managed-worktree.ts";
import { PolicyStore } from "../../permissions/policy-store.ts";
import { ShellAdapter } from "../../permissions/adapters/shell.ts";
import type { WorkspaceGrantSnapshot } from "../../permissions/approvals/workspace-store.ts";
import type { InteractionBroker } from "../interactions/interaction-broker.ts";
import {
  agentToolResource,
  permissionIntentForTool,
} from "./tool-intent.ts";
import type { JsonObject } from "../../protocol/validation.ts";

const EXTERNAL_TOOL_EXECUTION_PROFILE: ExecutionProfile = {
  backend: "none",
  filesystem: { read: ["*"], write: ["*"] },
  network: { allow: [{ host: "*" }] },
  environment: { inherit: [], set: {} },
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

export class PermissionService {
  private readonly policies = new PolicyStore();
  private readonly approvals = new PermissionApprovalBroker();
  private readonly kernel = new PermissionKernel(
    this.policies,
    this.approvals,
    new JsonlPermissionAuditLog(),
  );
  private readonly execution = new ExecutionBroker(
    this.kernel,
    new DirectRunner(),
  );
  private readonly pending = new Map<string, Authorization>();
  private readonly shellAdapter = new ShellAdapter();
  private readonly sessionIdProvider: () => string;
  private readonly cwdProvider: () => string;
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
  ) {
    this.interactions = interactions;
    this.send = send;
    this.planMode = planMode;
    this.isConnected = isConnected;
    this.sessionIdProvider = scope.sessionId;
    this.cwdProvider = scope.cwd;
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
    const toolName = event.toolName;
    const input = event.input as Record<string, unknown>;
    const path = typeof input.path === "string" ? input.path : undefined;
    const command =
      typeof input.command === "string" ? input.command : undefined;
    const agent = context.agent ?? {};
    const profile = agent.profileConfig;
    if (profile && !profile.tools.includes(toolName)) {
      return {
        block: true,
        reason: `Tool ${toolName} is not exposed to profile ${agent.profile}`,
      };
    }
    const profileEffect = profile
      ? agentPermissionEffect(
          profile,
          toolName,
          agentToolResource(context.cwd, path, command),
        )
      : undefined;
    const sessionId = agent.sessionId ?? this.tryCurrentScopeId();
    if (!sessionId) {
      return { block: true, reason: "Permission scope is unavailable" };
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
    if (profileEffect === "deny") {
      addToolRule(`profile-${agent.profile}-deny`, "deny", "managed");
    } else if (profileEffect === "ask") {
      addToolRule(`profile-${agent.profile}-ask`, "ask", "managed");
    }
    if (
      agent.planReadOnly &&
      intent.atoms.some(
        (atom) =>
          atom.kind === "exec" ||
          (atom.kind === "file" &&
            atom.operation !== "read" &&
            atom.operation !== "list") ||
          atom.kind === "opaque_code",
      )
    ) {
      addToolRule("plan-read-only", "deny", "managed");
    }
    if (agent.agentId && mutatesManagedWorktree(intent)) {
      addToolRule("managed-worktree-boundary", "deny", "managed");
    }
    if (
      !agent.agentId &&
      this.planMode.current() &&
      intent.atoms.some(
        (atom) =>
          atom.kind === "exec" ||
          (atom.kind === "file" &&
            atom.operation !== "read" &&
            atom.operation !== "list"),
      )
    ) {
      addToolRule("plan-mode-mutation", "deny", "managed");
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
        block: true,
        reason:
          authorization.evaluation.effect === "deny"
            ? "Denied by permission policy"
            : "Denied by user",
      };
    }
    if (
      !this.execution.beginExternalTool(
        authorization,
        normalize(),
        EXTERNAL_TOOL_EXECUTION_PROFILE,
      )
    ) {
      return { block: true, reason: "Tool input changed after approval" };
    }
    this.pending.set(event.toolCallId, authorization);
    return undefined;
  }

  finishTool(toolCallId: string, succeeded: boolean): void {
    const authorization = this.pending.get(toolCallId);
    if (!authorization) return;
    this.pending.delete(toolCallId);
    this.execution.finishExternalTool(
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
