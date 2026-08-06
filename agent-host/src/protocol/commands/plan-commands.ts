import type { LegacyHostOperations } from "../../legacy-host-operations.ts";
import {
  type CommandDefinition,
  requestObject,
} from "../command-definition.ts";
import { enumField } from "../validation.ts";

export function createPlanCommands(
  ops: LegacyHostOperations,
): CommandDefinition<any>[] {
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
      handle: (_context, request) => ops.setPlanMode(request.active),
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
      handle: (_context, request) => ops.executePlan(request.context),
    },
  ];
}
