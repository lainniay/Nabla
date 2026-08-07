import type {
  GrantBundle,
  PermissionAdapter,
  PermissionExplanation,
  PermissionIntent,
  ToolContext,
} from "../model.ts";
import {
  analyzeShell,
  planShell,
  type ExecutionPlan,
  type ShellAnalysis,
} from "../shell/planner.ts";
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
    const analysis = analyzeShell(input.command, cwd, environment);
    const plan = analysis.plan;
    return createIntent(
      context,
      "bash",
      { command: input.command, cwd, environment, plan, safety: analysis.safety },
      plan.atoms,
    );
  }

  plan(intent: PermissionIntent): ExecutionPlan {
    const input = intent.normalizedInput as { plan?: ExecutionPlan };
    if (!input.plan) throw new Error("Shell intent does not contain an execution plan");
    return input.plan;
  }

  analysis(intent: PermissionIntent): ShellAnalysis {
    const input = intent.normalizedInput as {
      plan?: ExecutionPlan;
      safety?: ShellAnalysis["safety"];
    };
    if (!input.plan || !input.safety) {
      throw new Error("Shell intent does not contain a shell analysis");
    }
    return { plan: input.plan, capabilities: input.plan.atoms, safety: input.safety };
  }

  proposeGrants(intent: PermissionIntent): GrantBundle[] {
    return exactGrantProposals(intent);
  }

  explain(intent: PermissionIntent): PermissionExplanation {
    return defaultExplanation(intent);
  }
}
