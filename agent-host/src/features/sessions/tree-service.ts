import {
  copyToClipboard,
  type SessionManager,
} from "@earendil-works/pi-coding-agent";

import type { RuntimeAccess } from "../../runtime/runtime-access.ts";
import type { PlanModeService } from "../../runtime/plan-mode-service.ts";
import type { PlanStore } from "../../plan.ts";
import {
  buildTreeSnapshot,
  copyTextForEntry,
  type TreeFilterMode,
  type TreeSnapshot,
} from "../../session-navigation.ts";
import { restorePlanMode } from "../../plan.ts";
import type { JsonObject } from "../../protocol/validation.ts";

export class TreeService {
  private readonly runtime: RuntimeAccess;
  private readonly planMode: PlanModeService;
  private readonly plans: PlanStore;
  private readonly send: (event: JsonObject) => void;
  private readonly activation: () => JsonObject;
  private readonly onTreeNavigation: () => void;

  constructor(
    runtime: RuntimeAccess,
    planMode: PlanModeService,
    plans: PlanStore,
    send: (event: JsonObject) => void,
    activation: () => JsonObject,
    onTreeNavigation: () => void,
  ) {
    this.runtime = runtime;
    this.planMode = planMode;
    this.plans = plans;
    this.send = send;
    this.activation = activation;
    this.onTreeNavigation = onTreeNavigation;
  }

  state(input: {
    filterMode: TreeFilterMode;
    query: string;
    foldedEntryIds: string[];
  }): TreeSnapshot {
    return buildTreeSnapshot(
      this.runtime.current().session.sessionManager,
      input.filterMode,
      input.query,
      input.foldedEntryIds,
    );
  }

  label(input: { entryId: string; label?: string }): void {
    const runtime = this.runtime.requireIdle("Cannot edit tree labels");
    runtime.session.sessionManager.appendLabelChange(input.entryId, input.label);
  }

  async copy(entryId: string): Promise<void> {
    const runtime = this.runtime.current();
    const entry = runtime.session.sessionManager.getEntry(entryId);
    if (!entry) throw new Error(`Tree entry not found: ${entryId}`);
    const text = copyTextForEntry(entry);
    if (!text) throw new Error("Selected tree entry has no text to copy");
    await copyToClipboard(text);
  }

  async navigate(input: {
    entryId: string;
    summarize: boolean;
    customInstructions?: string;
  }): Promise<JsonObject> {
    const generation = this.runtime.sessionGeneration();
    const runtime = this.runtime.requireIdle("Cannot navigate the tree");
    const result = await runtime.session.navigateTree(input.entryId, {
      summarize: input.summarize,
      ...(input.customInstructions
        ? { customInstructions: input.customInstructions }
        : {}),
      replaceInstructions: false,
    });
    if (this.runtime.sessionGeneration() !== generation) {
      return { cancelled: true, aborted: result.aborted === true };
    }
    if (result.cancelled) {
      return { cancelled: true, aborted: result.aborted === true };
    }
    this.restorePlanState(runtime.session.sessionManager);
    this.onTreeNavigation();
    return {
      cancelled: false,
      aborted: false,
      editorText: result.editorText,
      activation: this.activation(),
    };
  }

  abort(): void {
    this.runtime.current().session.abortBranchSummary();
  }

  private restorePlanState(sessionManager: SessionManager): void {
    const restored = this.plans.restore(sessionManager.getBranch());
    const restoredPlanMode = restorePlanMode(sessionManager.getBranch());
    const session = this.runtime.current().session;
    if (this.planMode.current() !== restoredPlanMode) {
      this.planMode.set(session, restoredPlanMode);
    }
    this.send({
      type: "plan_mode_state",
      active: this.planMode.current(),
      activeTools: session.getActiveToolNames(),
    });
    this.send({
      type: "plan_state",
      artifact: restored ?? null,
    });
  }

}
