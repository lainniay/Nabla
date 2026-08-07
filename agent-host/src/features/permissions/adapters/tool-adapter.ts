import { randomUUID } from "node:crypto";

import { proposeGrantBundles } from "../grant-proposal.ts";
import type {
  CapabilityAtom,
  GrantBundle,
  PermissionExplanation,
  PermissionIntent,
  ToolContext,
} from "../model.ts";
import { digestValue } from "../shell/digest.ts";

export function createIntent(
  context: ToolContext,
  tool: string,
  normalizedInput: unknown,
  atoms: CapabilityAtom[],
): PermissionIntent {
  const digest = digestValue({
    tool,
    normalizedInput,
    atoms,
    toolCallId: context.toolCallId,
    sessionId: context.sessionId,
    workspaceId: context.workspaceId,
  });
  return {
    id: randomUUID(),
    toolCallId: context.toolCallId,
    sessionId: context.sessionId,
    workspaceId: context.workspaceId,
    tool,
    normalizedInput,
    atoms,
    digest,
  };
}

export function exactGrantProposals(intent: PermissionIntent): GrantBundle[] {
  return proposeGrantBundles(intent);
}

export function defaultExplanation(
  intent: PermissionIntent,
): PermissionExplanation {
  return {
    summary: `${intent.tool} requests ${intent.atoms.length} capabilities`,
    details: intent.atoms.map((atom) => {
      if (atom.kind === "file") return `${atom.operation} ${atom.path}`;
      if (atom.kind === "exec") {
        return `execute ${[atom.executable, ...atom.argv].join(" ")}`;
      }
      if (atom.kind === "network") return `${atom.operation} ${atom.host}`;
      return `opaque ${atom.runtime} code (${atom.reason})`;
    }),
    risk: intent.atoms.some((atom) => atom.kind === "opaque_code")
      ? "high"
      : "normal",
  };
}
