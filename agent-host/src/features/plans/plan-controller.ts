import type {
  AgentSession,
  ModelRuntime,
} from "@earendil-works/pi-coding-agent";

import type { RuntimeAccess } from "../../runtime/runtime-access.ts";
import type { PermissionRule } from "../permissions/model.ts";
import { READ_ONLY_TOOL_NAMES } from "../permissions/shell/rules.ts";
import {
  PLAN_MODE_ENTRY_TYPE,
  type PlanArtifact,
  type PlanContent,
  type PlanSessionEntry,
} from "./model.ts";
import { PlanStore, restorePlanMode } from "./store.ts";
import {
  executePlan as dispatchPlanExecution,
  type PlanExecutionResult,
} from "./execution.ts";
import type { JsonObject } from "../../protocol/validation.ts";

export const PLAN_MODE_POLICY = {
  exposedTools: [
    ...READ_ONLY_TOOL_NAMES,
    "ask_user",
    "submit_plan",
    "delegate_task",
  ],
  standardTools: [
    ...READ_ONLY_TOOL_NAMES,
    "edit",
    "write",
    "bash",
    "delegate_task",
  ],
  permissionRules: [
    {
      id: "plan-mode-exec",
      effect: "deny",
      source: "managed",
      matcher: { kind: "exec", executable: "*" },
    },
    {
      id: "plan-mode-opaque",
      effect: "deny",
      source: "managed",
      matcher: { kind: "opaque_code", runtime: "*" },
    },
    {
      id: "plan-mode-network",
      effect: "deny",
      source: "managed",
      matcher: { kind: "network", operation: "connect", host: "*" },
    },
    ...(["create", "write", "truncate", "append", "rename", "delete"] as const)
      .map((operation) => ({
        id: `plan-mode-file-${operation}`,
        effect: "deny",
        source: "managed",
        matcher: { kind: "file", operation, path: "*" },
      })),
  ] as PermissionRule[],
} as const;

export interface PlanSnapshot {
  active: boolean;
  artifact: PlanArtifact | null;
}

export interface PlanModePort {
  current(): boolean;
  set(active: boolean): { active: boolean; activeTools: readonly string[] };
}

export class PlanController {
  private readonly store: PlanStore;
  private readonly modelRuntime: ModelRuntime;
  private readonly runtime: RuntimeAccess;
  private readonly send: (event: JsonObject) => void;
  private active = false;

  constructor(
    store: PlanStore,
    modelRuntime: ModelRuntime,
    runtime: RuntimeAccess,
    send: (event: JsonObject) => void,
  ) {
    this.store = store;
    this.modelRuntime = modelRuntime;
    this.runtime = runtime;
    this.send = send;
  }

  current(): boolean {
    return this.active;
  }

  state(): PlanSnapshot {
    return {
      active: this.active,
      artifact: this.store.latest() ?? null,
    };
  }

  snapshot(): PlanArtifact | null {
    return this.store.latest() ?? null;
  }

  submit(content: PlanContent, sessionId: string): PlanArtifact {
    return this.store.submit(content, sessionId);
  }

  activateSession(
    branch: readonly PlanSessionEntry[],
    session: AgentSession = this.runtime.current().session,
  ): PlanSnapshot {
    const artifact = this.store.restore(branch) ?? null;
    const restoredMode = restorePlanMode(branch);
    if (this.active !== restoredMode) {
      this.applyToSession(session, restoredMode);
      this.active = restoredMode;
    }
    this.send({ type: "plan_state", artifact });
    this.send({
      type: "plan_mode_state",
      active: this.active,
      activeTools: session.getActiveToolNames(),
    });
    return this.state();
  }

  onSessionActivated(
    branch: readonly PlanSessionEntry[],
  ): PlanArtifact | null {
    const artifact = this.store.restore(branch) ?? null;
    this.send({ type: "plan_state", artifact });
    return artifact;
  }

  setMode(active: boolean): {
    active: boolean;
    activeTools: readonly string[];
  } {
    const session = this.runtime.requireIdle("Cannot switch plan mode").session;
    const activeTools = this.applyToSession(session, active);
    this.active = active;
    session.sessionManager.appendCustomEntry(PLAN_MODE_ENTRY_TYPE, { active });
    this.send({ type: "plan_mode_state", active, activeTools });
    return { active, activeTools };
  }

  set(active: boolean): {
    active: boolean;
    activeTools: readonly string[];
  } {
    return this.setMode(active);
  }

  reapply(session: AgentSession): void {
    this.applyToSession(session, this.active);
    this.send({
      type: "plan_mode_state",
      active: this.active,
      activeTools: session.getActiveToolNames(),
    });
  }

  planState(): { scopeId: string; artifact: PlanArtifact | null } {
    return {
      scopeId: this.runtime.current().session.sessionId,
      artifact: this.snapshot(),
    };
  }

  async execute(mode: "current" | "fresh"): Promise<PlanExecutionResult> {
    return dispatchPlanExecution(mode, {
      plans: this.store,
      modelRuntime: this.modelRuntime,
      runtime: () => this.runtime.current(),
      setPlanMode: (active) => {
        this.setMode(active);
      },
      send: (message) => this.send(message),
      reportTurnError: (error) => {
        this.send({
          type: "host_warning",
          message: `Plan implementation turn failed: ${
            error instanceof Error ? error.message : String(error)
          }`,
        });
      },
    });
  }

  private applyToSession(session: AgentSession, active: boolean): string[] {
    const expected = [...toolsForPlanMode(active)];
    const previous = session.getActiveToolNames();
    session.setActiveToolsByName(expected);
    const activeTools = session.getActiveToolNames();
    const missing = expected.filter((tool) => !activeTools.includes(tool));
    if (missing.length > 0) {
      session.setActiveToolsByName(previous);
      throw new Error(`Pi did not register required tools: ${missing.join(", ")}`);
    }
    return activeTools;
  }
}

function toolsForPlanMode(active: boolean): readonly string[] {
  return active ? PLAN_MODE_POLICY.exposedTools : PLAN_MODE_POLICY.standardTools;
}
