import {
  type CommandDefinition,
  requestObject,
} from "../command-definition.ts";
import { enumField, stringField } from "../validation.ts";
import { THINKING_LEVELS } from "../../policy/tool-policy.ts";
import type { JsonObject } from "../validation.ts";

export interface ModelCommandPort {
  list(): Promise<{
    current: { provider: string; id: string } | null;
    models: Array<{
      provider: string;
      id: string;
      name: string;
      reasoning: unknown;
      contextWindow: unknown;
    }>;
  }>;
  set(input: {
    provider: string;
    modelId: string;
  }): Promise<{ provider: string; id: string; name: string }>;
  setThinking(level: (typeof THINKING_LEVELS)[number]): JsonObject;
}

export function createModelCommands(ops: ModelCommandPort): CommandDefinition<any>[] {
  return [
    {
      type: "model_list",
      lane: undefined,
      decode: requestObject,
      handle: () => ops.list(),
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
      handle: (_context, request) => ops.set(request),
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
