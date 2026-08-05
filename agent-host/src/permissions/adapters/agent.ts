import type {
  CapabilityGrantSet,
  CapabilityMatcher,
  GrantBundle,
  PermissionAdapter,
  PermissionExplanation,
  PermissionIntent,
  ToolContext,
} from "../model.ts";
import {
  createIntent,
  defaultExplanation,
  exactGrantProposals,
} from "./tool-adapter.ts";
import { digestValue } from "../shell/digest.ts";

export interface AgentInput {
  action: "spawn";
  profile?: string;
  grants?: CapabilityGrantSet;
  payload?: unknown;
}

export class AgentAdapter implements PermissionAdapter<AgentInput> {
  normalize(context: ToolContext, input: AgentInput): PermissionIntent {
    return createIntent(context, "agent", input, [{
      kind: "opaque_code",
      runtime: `agent:${input.action}`,
      digest: digestValue(input),
      reason: "delegated agent action",
    }]);
  }

  proposeGrants(intent: PermissionIntent): GrantBundle[] {
    return exactGrantProposals(intent).filter((bundle) => bundle.scope !== "workspace");
  }

  explain(intent: PermissionIntent): PermissionExplanation {
    return defaultExplanation(intent);
  }
}

export function intersectGrantSets(
  ...sets: readonly CapabilityGrantSet[]
): CapabilityGrantSet {
  if (sets.length === 0) return { matchers: [] };
  const [first, ...rest] = sets;
  return {
    matchers: first!.matchers.filter((matcher) =>
      rest.every((set) => set.matchers.some((candidate) =>
        matcherKey(candidate) === matcherKey(matcher))),
    ),
  };
}

function matcherKey(matcher: CapabilityMatcher): string {
  return JSON.stringify(matcher);
}
