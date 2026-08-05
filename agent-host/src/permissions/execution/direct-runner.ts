import { spawn } from "node:child_process";

import type { ExecCapability, ExecutionProfile } from "../model.ts";
import type {
  ExecutionResult,
  SandboxBackend,
} from "./sandbox-backend.ts";

export class DirectRunner implements SandboxBackend {
  readonly kind = "none" as const;

  run(
    command: ExecCapability,
    profile: ExecutionProfile,
    signal?: AbortSignal,
  ): Promise<ExecutionResult> {
    if (profile.backend !== "none") {
      throw new Error(`DirectRunner cannot provide ${profile.backend} isolation`);
    }
    return new Promise((resolveResult, reject) => {
      const child = spawn(command.executable, command.argv, {
        cwd: command.cwd,
        env: {
          ...selectEnvironment(process.env, profile.environment.inherit),
          ...profile.environment.set,
          ...command.environment,
        },
        shell: false,
        signal,
        stdio: ["ignore", "pipe", "pipe"],
      });
      let stdout = "";
      let stderr = "";
      child.stdout.setEncoding("utf8");
      child.stderr.setEncoding("utf8");
      child.stdout.on("data", (chunk: string) => {
        stdout += chunk;
      });
      child.stderr.on("data", (chunk: string) => {
        stderr += chunk;
      });
      child.once("error", reject);
      child.once("close", (exitCode, childSignal) => {
        resolveResult({
          exitCode,
          ...(childSignal ? { signal: childSignal } : {}),
          stdout,
          stderr,
        });
      });
    });
  }
}

function selectEnvironment(
  source: NodeJS.ProcessEnv,
  names: readonly string[],
): Record<string, string> {
  return Object.fromEntries(
    names.flatMap((name) => {
      const value = source[name];
      return value === undefined ? [] : [[name, value]];
    }),
  );
}
