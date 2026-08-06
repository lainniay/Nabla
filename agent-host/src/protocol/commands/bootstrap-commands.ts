import type { LegacyHostOperations } from "../../legacy-host-operations.ts";
import {
  type CommandDefinition,
  requestObject,
} from "../command-definition.ts";

export function createBootstrapCommands(
  ops: LegacyHostOperations,
): CommandDefinition<any>[] {
  return [
    {
      type: "bootstrap_state",
      lane: undefined,
      decode: requestObject,
      handle: () => ops.bootstrapState(),
    },
  ];
}
