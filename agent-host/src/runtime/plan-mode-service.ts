import type { AgentSession } from "@earendil-works/pi-coding-agent";

import { READ_ONLY_TOOL_NAMES } from "../policy/tool-policy.ts";

export const PLAN_TOOLS = [
  ...READ_ONLY_TOOL_NAMES,
  "ask_user",
  "submit_plan",
  "delegate_task",
] as const;

export const STANDARD_TOOLS = [
  ...READ_ONLY_TOOL_NAMES,
  "edit",
  "write",
  "bash",
  "delegate_task",
] as const;

export class PlanModeService {
  private active = false;

  current(): boolean {
    return this.active;
  }

  apply(session: AgentSession): void {
    this.applyToSession(session, this.active);
  }

  restore(session: AgentSession, active: boolean): readonly string[] {
    const activeTools = this.applyToSession(session, active);
    this.active = active;
    return activeTools;
  }

  set(session: AgentSession, active: boolean): readonly string[] {
    if (!session.isIdle) {
      throw new Error("Cannot switch mode while the agent is running");
    }
    const activeTools = this.applyToSession(session, active);
    this.active = active;
    return activeTools;
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
  return active ? PLAN_TOOLS : STANDARD_TOOLS;
}
