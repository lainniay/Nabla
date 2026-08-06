import type { LegacyHostOperations } from "../../legacy-host-operations.ts";
import {
  type CommandDefinition,
  requestObject,
} from "../command-definition.ts";
import { enumField, stringField } from "../validation.ts";

export function createAgentCommands(
  ops: LegacyHostOperations,
): CommandDefinition<any>[] {
  return [
    {
      type: "agents_state",
      lane: undefined,
      decode: requestObject,
      handle: () => ops.agentsSnapshot(),
    },
    {
      type: "subagent_start",
      lane: "subagents",
      decode: (value) => {
        const request = requestObject(value);
        const task = stringField(request, "task").trim();
        if (!task) throw new Error("Subagent task must not be empty");
        return {
          profile: stringField(request, "profile"),
          task,
        };
      },
      handle: (_context, request) => ops.startSubagent(request),
    },
    {
      type: "subagent_cancel",
      lane: "subagents",
      decode: (value) => {
        const request = requestObject(value);
        return {
          agentId: stringField(request, "agentId"),
        };
      },
      handle: (_context, request) => ops.cancelSubagent(request.agentId),
    },
    {
      type: "subagent_integrate",
      lane: (request) =>
        `integration:${
          typeof request.agentId === "string" ? request.agentId : "unknown"
        }`,
      decode: (value) => {
        const request = requestObject(value);
        return {
          agentId: stringField(request, "agentId"),
          action: enumField(
            request,
            "action",
            ["apply", "resolve", "keep", "discard"] as const,
          ),
        };
      },
      handle: (_context, request) => ops.integrateSubagent(request),
    },
  ];
}
