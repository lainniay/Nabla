import {
  type CommandDefinition,
  requestObject,
} from "../command-definition.ts";
import type { BootstrapState } from "../contracts.ts";

export function createBootstrapCommands(ops: {
  snapshot(): BootstrapState;
}): CommandDefinition<any>[] {
  return [
    {
      type: "bootstrap_state",
      lane: undefined,
      decode: requestObject,
      handle: () => ops.snapshot(),
    },
  ];
}
