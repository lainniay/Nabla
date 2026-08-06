import {
  ContextBudgetManager,
  type ContextSnapshot,
} from "../../context-manager.ts";
import type { JsonObject } from "../../protocol/validation.ts";

export class ContextService {
  private readonly budget: ContextBudgetManager;
  private readonly send: (event: JsonObject) => void;
  private readonly contextSnapshot: (
    snapshot: ContextSnapshot,
  ) => ContextSnapshot;

  constructor(
    budget: ContextBudgetManager,
    send: (event: JsonObject) => void,
    contextSnapshot: (snapshot: ContextSnapshot) => ContextSnapshot,
  ) {
    this.budget = budget;
    this.send = send;
    this.contextSnapshot = contextSnapshot;
  }

  snapshot(): ContextSnapshot {
    return this.budget.snapshot();
  }

  scopedSnapshot(): ContextSnapshot {
    return this.contextSnapshot(this.budget.snapshot());
  }

  onSessionStart(sessionId: string): ContextSnapshot {
    return this.budget.onSessionStart(sessionId);
  }

  onModelResponse(usage: Parameters<ContextBudgetManager["onModelResponse"]>[0]): ContextSnapshot {
    return this.budget.onModelResponse(usage);
  }

  filter(
    messages: Parameters<ContextBudgetManager["filter"]>[0],
    usage: Parameters<ContextBudgetManager["filter"]>[1],
    options: Parameters<ContextBudgetManager["filter"]>[2],
  ): ReturnType<ContextBudgetManager["filter"]> {
    return this.budget.filter(messages, usage, options);
  }

  onCompaction(
    record: Parameters<ContextBudgetManager["onCompaction"]>[0],
  ): ContextSnapshot {
    return this.budget.onCompaction(record);
  }

  onTreeNavigation(): ContextSnapshot {
    return this.budget.onTreeNavigation();
  }

  publish(snapshot: ContextSnapshot): void {
    const policyWarning = this.budget.takeWarning();
    this.send({
      type: "context_budget",
      snapshot: this.contextSnapshot(snapshot),
      ...(policyWarning ? { policyWarning } : {}),
    });
  }

  onRuntimeSessionStart(runtime: {
    sessionManager: { getSessionId(): string };
    getContextUsage(): Parameters<
      ContextBudgetManager["onModelResponse"]
    >[0];
  }): void {
    this.onSessionStart(runtime.sessionManager.getSessionId());
    this.publish(this.onModelResponse(runtime.getContextUsage()));
  }
}
