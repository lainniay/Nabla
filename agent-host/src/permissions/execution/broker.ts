import type { ShellAdapter, ShellInput } from "../adapters/shell.ts";
import type { Authorization, PermissionKernel } from "../kernel.ts";
import type {
  ExecutionProfile,
  PermissionIntent,
  ToolContext,
} from "../model.ts";
import type { SandboxBackend, ExecutionResult } from "./sandbox-backend.ts";
import { ShellFallback } from "./shell-fallback.ts";

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
    if (!this.kernel.consumeForExecution(authorization, recomputed)) {
      throw new Error("Permission changed or was not granted before execution");
    }
    const plan = adapter.plan(recomputed);
    if (plan.opaque || plan.requiresShell) {
      const normalized = recomputed.normalizedInput as {
        cwd: string;
        command: string;
      };
      return [await this.shellFallback.run(
        "/bin/sh",
        normalized.command,
        normalized.cwd,
        profile,
        signal,
      )];
    }
    const results: ExecutionResult[] = [];
    for (const command of plan.commands) {
      results.push(await this.backend.run(command, profile, signal));
    }
    return results;
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
