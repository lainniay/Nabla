import {
  type CommandDefinition,
  requestObject,
} from "../command-definition.ts";
import { stringField } from "../validation.ts";
import type { WorkspaceGrantSnapshot } from "../../permissions/approvals/workspace-store.ts";

export interface PermissionCommandPort {
  workspaceRules(): WorkspaceGrantSnapshot;
  revokeWorkspaceRule(ruleId: string): WorkspaceGrantSnapshot;
  clearWorkspaceRules(): WorkspaceGrantSnapshot;
}

export function createPermissionCommands(
  ops: PermissionCommandPort,
): CommandDefinition<any>[] {
  return [
    {
      type: "approval_rules",
      lane: "configuration",
      decode: requestObject,
      handle: () => ops.workspaceRules(),
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
      handle: (_context, request) => ops.revokeWorkspaceRule(request.ruleId),
    },
    {
      type: "approval_rules_clear",
      lane: "configuration",
      decode: requestObject,
      handle: () => ops.clearWorkspaceRules(),
    },
  ];
}
