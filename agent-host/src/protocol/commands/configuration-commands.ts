import type { LegacyHostOperations } from "../../legacy-host-operations.ts";
import {
  type CommandDefinition,
  requestObject,
} from "../command-definition.ts";

export function createConfigurationCommands(
  ops: LegacyHostOperations,
): CommandDefinition<any>[] {
  return [
    {
      type: "resource_state",
      lane: undefined,
      decode: requestObject,
      handle: () => ops.resourceSnapshot(),
    },
    {
      type: "resource_reload",
      lane: "configuration",
      decode: requestObject,
      handle: () => ops.reloadResources(),
    },
    {
      type: "workspace_trust",
      lane: "configuration",
      decode: (value) => {
        const request = requestObject(value);
        if (typeof request.trusted !== "boolean") {
          throw new Error("workspace_trust requires a boolean trusted field");
        }
        return { trusted: request.trusted };
      },
      handle: (_context, request) => ops.setWorkspaceTrust(request.trusted),
    },
    {
      type: "agents_reload",
      lane: "configuration",
      decode: requestObject,
      handle: () => ops.reloadAgents(),
    },
  ];
}
