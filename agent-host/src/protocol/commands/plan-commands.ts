import {
  type CommandDefinition,
  requestObject,
} from "../command-definition.ts";
import { enumField } from "../validation.ts";
import type { PlanArtifact } from "../../plan.ts";
import type { PlanExecutionResult } from "../../plan-execution.ts";

export interface PlanCommandPort {
  setMode(active: boolean): {
    active: boolean;
    activeTools: readonly string[];
  };
  planState(): { scopeId: string; artifact: PlanArtifact | null };
  execute(mode: "current" | "fresh"): Promise<PlanExecutionResult>;
}

export function createPlanCommands(ops: PlanCommandPort): CommandDefinition<any>[] {
  return [
    {
      type: "set_plan_mode",
      lane: "session",
      decode: (value) => {
        const request = requestObject(value);
        if (typeof request.active !== "boolean") {
          throw new Error("set_plan_mode requires a boolean active field");
        }
        return { active: request.active };
      },
      handle: (_context, request) => ops.setMode(request.active),
    },
    {
      type: "get_plan_state",
      lane: undefined,
      decode: requestObject,
      handle: () => ops.planState(),
    },
    {
      type: "plan_execute",
      lane: "session",
      decode: (value) => {
        const request = requestObject(value);
        return {
          context: enumField(request, "context", ["current", "fresh"] as const),
        };
      },
      handle: (_context, request) => ops.execute(request.context),
    },
  ];
}
