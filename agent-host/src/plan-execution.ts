import type {
  AgentSessionRuntime,
  ModelRuntime,
} from "@earendil-works/pi-coding-agent";

import { estimateTextTokens } from "./context-manager.ts";
import {
  PLAN_ENTRY_TYPE,
  PLAN_MODE_ENTRY_TYPE,
  type PlanArtifact,
  type PlanStore,
  planImplementationPrompt,
} from "./plan.ts";
import type { JsonObject } from "./protocol/validation.ts";

export type PlanExecutionContext = "current" | "fresh";

export interface PlanExecutionResult {
  sessionId: string;
  context: PlanExecutionContext;
}

export interface PlanExecutionDeps {
  plans: PlanStore;
  modelRuntime: ModelRuntime;
  runtime: () => AgentSessionRuntime;
  setPlanMode: (active: boolean) => void;
  send: (message: JsonObject) => void;
  reportTurnError: (error: unknown) => void;
}

export const PLAN_TRANSFER_MAX_ABSOLUTE_TOKENS = 24_000;
const PLAN_TRANSFER_MAX_CONTEXT_FRACTION = 0.25;

export function transferBudget(
  contextWindow: number | null | undefined,
): number {
  if (
    contextWindow === null ||
    contextWindow === undefined ||
    contextWindow <= 0
  ) {
    return PLAN_TRANSFER_MAX_ABSOLUTE_TOKENS;
  }
  return Math.min(
    PLAN_TRANSFER_MAX_ABSOLUTE_TOKENS,
    Math.floor(contextWindow * PLAN_TRANSFER_MAX_CONTEXT_FRACTION),
  );
}

export function freshImplementationPrompt(artifact: PlanArtifact): string {
  return [
    "You are implementing an approved Nabla plan in a fresh session.",
    "The planning transcript is not available in this session.",
    "Re-check repository state before editing because files may have changed.",
    "Treat the plan as implementation guidance, not an immutable workflow.",
    "Report material deviations from the plan.",
    "",
    planImplementationPrompt(artifact),
  ].join("\n");
}

export async function executePlan(
  context: PlanExecutionContext,
  deps: PlanExecutionDeps,
): Promise<PlanExecutionResult> {
  const artifact = deps.plans.latest();
  if (!artifact) throw new Error("No Plan is submitted");

  const runtime = deps.runtime();
  if (!runtime.session.isIdle) {
    throw new Error("Cannot execute a plan while the agent is running");
  }

  const implementationPrompt = planImplementationPrompt(artifact);
  let freshPrompt: string | undefined;
  if (context === "fresh") {
    const model = runtime.session.model;
    if (!model) {
      throw new Error(
        "Cannot inherit a model for a fresh execution session: no model is selected",
      );
    }
    if (!deps.modelRuntime.getModel(model.provider, model.id)) {
      throw new Error(
        `Inherited model ${model.provider}/${model.id} is unavailable in a new session`,
      );
    }
    freshPrompt = freshImplementationPrompt(artifact);
    const allowed = transferBudget(model.contextWindow);
    const estimated = estimateTextTokens(freshPrompt);
    if (estimated > allowed) {
      throw new Error(
        `Plan transfer needs ~${estimated} tokens but the fresh context budget allows ${allowed}. Return to Plan mode, shorten the Plan, and resubmit.`,
      );
    }
  }

  deps.setPlanMode(false);
  runtime.session.sessionManager.appendCustomEntry(PLAN_MODE_ENTRY_TYPE, {
    active: false,
  });
  deps.send({
    type: "plan_mode_state",
    active: false,
    activeTools: runtime.session.getActiveToolNames(),
  });

  if (context === "current") {
    void runtime.session
      .prompt(implementationPrompt)
      .catch(deps.reportTurnError);
    return { sessionId: runtime.session.sessionId, context };
  }

  const model = runtime.session.model;
  if (!model) {
    throw new Error(
      "Cannot inherit a model for the fresh execution session: no model is selected",
    );
  }
  const thinkingLevel = runtime.session.thinkingLevel;
  const parentSession = runtime.session.sessionFile;
  const result = await runtime.newSession({
    ...(parentSession ? { parentSession } : {}),
    setup: async (sessionManager) => {
      sessionManager.appendCustomEntry(PLAN_ENTRY_TYPE, artifact);
      sessionManager.appendCustomEntry(PLAN_MODE_ENTRY_TYPE, { active: false });
    },
  });
  if (result.cancelled) {
    throw new Error("Creating a fresh execution session was cancelled");
  }

  const target = runtime.session;
  await target.setModel(model);
  target.setThinkingLevel(thinkingLevel);
  deps.plans.adopt(artifact);
  deps.send({ type: "plan_state", artifact });
  void target.prompt(freshPrompt!).catch(deps.reportTurnError);
  return { sessionId: target.sessionId, context };
}
