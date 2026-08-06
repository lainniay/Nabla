import type { LegacyHostOperations } from "../../legacy-host-operations.ts";
import {
  type CommandDefinition,
  requestObject,
} from "../command-definition.ts";
import { enumField, stringField } from "../validation.ts";
import { THINKING_LEVELS } from "../../policy/tool-policy.ts";

export function createModelCommands(
  ops: LegacyHostOperations,
): CommandDefinition<any>[] {
  return [
    {
      type: "model_list",
      lane: undefined,
      decode: requestObject,
      handle: () => ops.listModels(),
    },
    {
      type: "model_set",
      lane: "session",
      decode: (value) => {
        const request = requestObject(value);
        return {
          provider: stringField(request, "provider"),
          modelId: stringField(request, "modelId"),
        };
      },
      handle: (_context, request) => ops.setModel(request),
    },
    {
      type: "thinking_set",
      lane: "session",
      decode: (value) => {
        const request = requestObject(value);
        return {
          level: enumField(request, "level", THINKING_LEVELS),
        };
      },
      handle: (_context, request) => ops.setThinking(request.level),
    },
  ];
}
