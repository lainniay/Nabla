import type { ExecutionProfile } from "../model.ts";
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
    shell: string,
    source: string,
    cwd: string,
    profile: ExecutionProfile,
    signal?: AbortSignal,
  ): Promise<ExecutionResult> {
    return this.runner.run(
      {
        kind: "exec",
        executable: shell,
        argv: ["-c", source],
        cwd,
        environment: {},
      },
      profile,
      signal,
    );
  }
}
