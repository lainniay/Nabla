import type { ShellAdapter, ShellInput } from "../adapters/shell.ts";
import type { Authorization, PermissionKernel } from "../kernel.ts";
import type {
  ExecutionProfile,
  PermissionIntent,
  ToolContext,
} from "../model.ts";
import type { SandboxBackend, ExecutionResult } from "./sandbox-backend.ts";
import { ShellFallback } from "./shell-fallback.ts";
import type { ExecutionPlan } from "../shell/planner.ts";

export interface AuthorizedExecutionPlan extends ExecutionPlan {
  executionProfile: ExecutionProfile;
}

export class ExecutionBroker {
  private readonly kernel: PermissionKernel;
  private readonly backend: SandboxBackend;
  private readonly shellFallback: ShellFallback;

  constructor(
    kernel: PermissionKernel,
    backend: SandboxBackend,
    shellFallback = new ShellFallback(backend),
  ) {
    this.kernel = kernel;
    this.backend = backend;
    this.shellFallback = shellFallback;
  }

  beginExternalTool(
    authorization: Authorization,
    recomputedIntent: PermissionIntent,
    profile: ExecutionProfile,
  ): boolean {
    if (profile.backend !== this.backend.kind) {
      return false;
    }
    return this.kernel.consumeForExecution(
      authorization,
      recomputedIntent,
      profile,
    );
  }

  finishExternalTool(
    authorization: Authorization,
    profile: ExecutionProfile,
    succeeded: boolean,
  ): void {
    this.kernel.recordExecutionResult(authorization, profile, succeeded);
  }

  async executeShell(
    authorization: Authorization,
    adapter: ShellAdapter,
    context: ToolContext,
    input: ShellInput,
    profile: ExecutionProfile,
    signal?: AbortSignal,
  ): Promise<ExecutionResult[]> {
    const recomputed = adapter.normalize(context, input);
    this.assertUnchanged(authorization.intent, recomputed);
    const shellPlan = adapter.plan(recomputed);
    const plan: AuthorizedExecutionPlan = {
      ...shellPlan,
      executionProfile: profile,
    };
    if (plan.executionProfile.backend !== this.backend.kind) {
      throw new Error(
        `Execution profile ${plan.executionProfile.backend} does not match ` +
          `backend ${this.backend.kind}`,
      );
    }
    if (
      !this.kernel.consumeForExecution(
        authorization,
        recomputed,
        plan.executionProfile,
      )
    ) {
      throw new Error("Permission changed or was not granted before execution");
    }
    try {
      if (plan.opaque || plan.requiresShell) {
        const result = await this.shellFallback.run(plan, signal);
        this.kernel.recordExecutionResult(
          authorization,
          plan.executionProfile,
          result.exitCode === 0,
        );
        return [result];
      }
      const results: ExecutionResult[] = [];
      for (const command of plan.commands) {
        results.push(
          await this.backend.run(command, plan.executionProfile, signal),
        );
      }
      this.kernel.recordExecutionResult(
        authorization,
        plan.executionProfile,
        results.every((result) => result.exitCode === 0),
      );
      return results;
    } catch (error) {
      this.kernel.recordExecutionResult(
        authorization,
        plan.executionProfile,
        false,
      );
      throw error;
    }
  }

  private assertUnchanged(
    approved: PermissionIntent,
    recomputed: PermissionIntent,
  ): void {
    if (approved.digest !== recomputed.digest) {
      throw new Error("cwd, argv, input, or environment changed after approval");
    }
  }
}
