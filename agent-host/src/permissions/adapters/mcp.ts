import type {
  GrantBundle,
  PermissionAdapter,
  PermissionExplanation,
  PermissionIntent,
  ToolContext,
} from "../model.ts";
import { digestValue } from "../shell/digest.ts";
import {
  createIntent,
  defaultExplanation,
  exactGrantProposals,
} from "./tool-adapter.ts";

export interface McpInput {
  server: string;
  method: string;
  arguments?: unknown;
}

export class McpAdapter implements PermissionAdapter<McpInput> {
  normalize(context: ToolContext, input: McpInput): PermissionIntent {
    const normalizedInput = {
      server: input.server,
      method: input.method,
      arguments: input.arguments ?? null,
    };
    return createIntent(context, "mcp", normalizedInput, [{
      kind: "opaque_code",
      runtime: `mcp:${input.server}/${input.method}`,
      digest: digestValue(normalizedInput),
      reason: "MCP method effects are declared by the server, not the host",
    }]);
  }

  proposeGrants(intent: PermissionIntent): GrantBundle[] {
    return exactGrantProposals(intent);
  }

  explain(intent: PermissionIntent): PermissionExplanation {
    return defaultExplanation(intent);
  }
}
