import type { ModelRuntime } from "@earendil-works/pi-coding-agent";

import type { RuntimeAccess } from "../../runtime/runtime-access.ts";
import type { PlanModeService } from "../../runtime/plan-mode-service.ts";
import {
  PlanStore,
  type PlanArtifact,
  type PlanContent,
  type PlanSessionEntry,
  PLAN_MODE_ENTRY_TYPE,
} from "../../plan.ts";
import {
  executePlan as dispatchPlanExecution,
  type PlanExecutionResult,
} from "../../plan-execution.ts";
import type { JsonObject } from "../../protocol/validation.ts";

export class PlanService {
  private readonly store: PlanStore;
  private readonly modelRuntime: ModelRuntime;
  private readonly runtime: RuntimeAccess;
  private readonly planMode: PlanModeService;
  private readonly send: (event: JsonObject) => void;

  constructor(
    store: PlanStore,
    modelRuntime: ModelRuntime,
    runtime: RuntimeAccess,
    planMode: PlanModeService,
    send: (event: JsonObject) => void,
  ) {
    this.store = store;
    this.modelRuntime = modelRuntime;
    this.runtime = runtime;
    this.planMode = planMode;
    this.send = send;
  }

  snapshot(): PlanArtifact | null {
    return this.store.latest() ?? null;
  }

  submit(content: PlanContent, sessionId: string): PlanArtifact {
    return this.store.submit(content, sessionId);
  }

  restore(entries: readonly PlanSessionEntry[]): PlanArtifact | null {
    return this.store.restore(entries) ?? null;
  }

  onSessionActivated(entries: readonly PlanSessionEntry[]): PlanArtifact | null {
    const restored = this.restore(entries);
    this.send({ type: "plan_state", artifact: restored });
    return restored;
  }

  async execute(mode: "current" | "fresh"): Promise<PlanExecutionResult> {
    return dispatchPlanExecution(mode, {
      plans: this.store,
      modelRuntime: this.modelRuntime,
      runtime: () => this.runtime.current(),
      setPlanMode: (active) =>
        this.planMode.set(this.runtime.current().session, active),
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

  setMode(active: boolean): {
    active: boolean;
    activeTools: readonly string[];
  } {
    const session = this.runtime.current().session;
    const activeTools = this.planMode.set(session, active);
    session.sessionManager.appendCustomEntry(PLAN_MODE_ENTRY_TYPE, {
      active,
    });
    const state = { active, activeTools };
    this.send({ type: "plan_mode_state", ...state });
    return state;
  }

  planState(): { scopeId: string; artifact: PlanArtifact | null } {
    return {
      scopeId: this.runtime.current().session.sessionId,
      artifact: this.snapshot(),
    };
  }
}
