import type { LegacyHostOperations } from "../../legacy-host-operations.ts";
import {
  type CommandDefinition,
  requestObject,
} from "../command-definition.ts";
import {
  enumField,
  stringField,
} from "../validation.ts";

export function createAuthCommands(
  ops: LegacyHostOperations,
): CommandDefinition<any>[] {
  return [
    {
      type: "auth_list",
      lane: undefined,
      decode: requestObject,
      handle: async () => ({ providers: await ops.listProviders() }),
    },
    {
      type: "auth_login",
      lane: "auth",
      decode: (value) => {
        const request = requestObject(value);
        return {
          flowId: stringField(request, "flowId"),
          providerId: stringField(request, "providerId"),
          authType: enumField(request, "authType", ["oauth", "api_key"] as const),
        };
      },
      handle: (context, request) => {
        if (!context.requestId) throw new Error("auth_login requires an id");
        return ops.startLogin(request);
      },
    },
    {
      type: "auth_reply",
      lane: "auth",
      decode: (value) => {
        const request = requestObject(value);
        return {
          flowId: stringField(request, "flowId"),
          promptId: stringField(request, "promptId"),
          value: stringField(request, "value"),
        };
      },
      handle: (_context, request) => ops.replyToPrompt(request),
    },
    {
      type: "auth_cancel",
      lane: "auth",
      decode: requestObject,
      handle: () => ops.cancelLogin(),
    },
    {
      type: "auth_logout",
      lane: "auth",
      decode: (value) => {
        const request = requestObject(value);
        return {
          providerId: stringField(request, "providerId"),
        };
      },
      handle: (_context, request) => ops.logout(request.providerId),
    },
  ];
}
