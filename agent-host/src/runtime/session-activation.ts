import type { AgentSessionRuntime } from "@earendil-works/pi-coding-agent";

import type { ContextSnapshot } from "../features/context/model.ts";
import type { PlanArtifact } from "../features/plans/model.ts";
import { projectSessionHistory } from "../features/sessions/history.ts";
import type { JsonObject } from "../protocol/validation.ts";
import type { PlanModePort } from "../features/plans/plan-controller.ts";

export function sessionActivation(
  runtime: AgentSessionRuntime,
  planMode: PlanModePort,
  plan: PlanArtifact | null,
  context: () => ContextSnapshot,
): JsonObject {
  const session = runtime.session;
  const manager = session.sessionManager;
  return {
    state: {
      model: session.model,
      thinkingLevel: session.thinkingLevel,
      isStreaming: session.isStreaming,
      isCompacting: session.isCompacting,
      steeringMode: session.steeringMode,
      followUpMode: session.followUpMode,
      sessionFile: session.sessionFile,
      sessionId: session.sessionId,
      sessionName: session.sessionName,
      autoCompactionEnabled: session.autoCompactionEnabled,
      messageCount: session.messages.length,
      pendingMessageCount: session.pendingMessageCount,
    },
    cwd: manager.getCwd(),
    planMode: planMode.current(),
    history: projectSessionHistory(manager.buildContextEntries()),
    plan,
    context: context(),
  };
}
