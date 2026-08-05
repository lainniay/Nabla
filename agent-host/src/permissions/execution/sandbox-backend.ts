import type {
  ExecCapability,
  ExecutionProfile,
} from "../model.ts";

export interface ExecutionResult {
  exitCode: number | null;
  signal?: NodeJS.Signals;
  stdout: string;
  stderr: string;
}

export interface SandboxBackend {
  readonly kind: "none" | "native" | "container";
  run(
    command: ExecCapability,
    profile: ExecutionProfile,
    signal?: AbortSignal,
  ): Promise<ExecutionResult>;
}
