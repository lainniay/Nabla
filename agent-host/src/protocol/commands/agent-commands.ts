import {
  type CommandDefinition,
  requestObject,
} from "../command-definition.ts";
import { enumField, stringField } from "../validation.ts";
import type { ActiveAgentSnapshot, AgentsSnapshot } from "../contracts.ts";
import type { JsonObject } from "../validation.ts";

export interface AgentCommandPort {
  agentsState(): AgentsSnapshot;
  startSubagent(input: {
    profile: string;
    task: string;
  }): { accepted: boolean; agent: ActiveAgentSnapshot };
  cancelSubagent(agentId: string): Promise<void>;
  integrateSubagent(input: {
    agentId: string;
    action: "apply" | "resolve" | "keep" | "discard";
  }): Promise<JsonObject>;
}

export function createAgentCommands(ops: AgentCommandPort): CommandDefinition<any>[] {
  return [
    {
      type: "agents_state",
      lane: undefined,
      decode: requestObject,
      handle: () => ops.agentsState(),
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
