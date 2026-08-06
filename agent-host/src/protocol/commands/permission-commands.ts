import type { LegacyHostOperations } from "../../legacy-host-operations.ts";
import {
  type CommandDefinition,
  requestObject,
} from "../command-definition.ts";
import { stringField } from "../validation.ts";

export function createPermissionCommands(
  ops: LegacyHostOperations,
): CommandDefinition<any>[] {
  return [
    {
      type: "approval_rules",
      lane: "configuration",
      decode: requestObject,
      handle: () => ops.workspaceApprovalRules(),
    },
    {
      type: "approval_rule_revoke",
      lane: "configuration",
      decode: (value) => {
        const request = requestObject(value);
        return {
          ruleId: stringField(request, "ruleId"),
        };
      },
      handle: (_context, request) => ops.revokeApprovalRule(request.ruleId),
    },
    {
      type: "approval_rules_clear",
      lane: "configuration",
      decode: requestObject,
      handle: () => ops.clearApprovalRules(),
    },
  ];
}
