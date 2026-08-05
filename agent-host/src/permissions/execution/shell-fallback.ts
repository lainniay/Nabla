import type { AuthorizedExecutionPlan } from "./broker.ts";
import type {
  ExecutionResult,
  SandboxBackend,
} from "./sandbox-backend.ts";
import { DirectRunner } from "./direct-runner.ts";

export class ShellFallback {
  private readonly runner: SandboxBackend;

  constructor(runner: SandboxBackend = new DirectRunner()) {
    this.runner = runner;
  }

  run(
    plan: AuthorizedExecutionPlan,
    signal?: AbortSignal,
  ): Promise<ExecutionResult> {
    return this.runner.run(
      {
        kind: "exec",
        executable: "/bin/sh",
        argv: ["-c", plan.source],
        cwd: plan.cwd,
        environment: {},
      },
      plan.executionProfile,
      signal,
    );
  }
}
