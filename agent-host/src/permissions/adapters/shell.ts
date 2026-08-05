import type {
  GrantBundle,
  PermissionAdapter,
  PermissionExplanation,
  PermissionIntent,
  ToolContext,
} from "../model.ts";
import { planShell, type ExecutionPlan } from "../shell/planner.ts";
import {
  createIntent,
  defaultExplanation,
  exactGrantProposals,
} from "./tool-adapter.ts";

export interface ShellInput {
  command: string;
  cwd?: string;
  environment?: Record<string, string>;
}

export class ShellAdapter implements PermissionAdapter<ShellInput> {
  normalize(context: ToolContext, input: ShellInput): PermissionIntent {
    const cwd = input.cwd ?? context.cwd;
    const environment = { ...context.environment, ...input.environment };
    const plan = planShell(input.command, cwd, environment);
    return createIntent(
      context,
      "bash",
      { command: input.command, cwd, environment, plan },
      plan.atoms,
    );
  }

  plan(intent: PermissionIntent): ExecutionPlan {
    const input = intent.normalizedInput as { plan?: ExecutionPlan };
    if (!input.plan) throw new Error("Shell intent does not contain an execution plan");
    return input.plan;
  }

  proposeGrants(intent: PermissionIntent): GrantBundle[] {
    return exactGrantProposals(intent);
  }

  explain(intent: PermissionIntent): PermissionExplanation {
    return defaultExplanation(intent);
  }
}
