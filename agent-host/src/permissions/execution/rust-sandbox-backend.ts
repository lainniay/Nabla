import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";

import {
  createLocalBashOperations,
  type BashOperations,
} from "@earendil-works/pi-coding-agent";

import type { SandboxStatus } from "../../protocol/contracts.ts";
import type { SandboxCapability } from "./sandbox-capability.ts";
import { disabledSandbox } from "./sandbox-capability.ts";
import type { SandboxExecutionProfile } from "./sandbox-profile.ts";

const DEFAULT_EXECUTABLE = process.env.NABLA_EXECUTABLE ?? "nabla";

export class RustSandboxBackend {
  readonly capability: SandboxCapability;
  private readonly executable: string;

  constructor(
    capability: SandboxCapability,
    executable: string = DEFAULT_EXECUTABLE,
  ) {
    this.capability = capability;
    this.executable = executable;
  }

  static async probe(
    executable: string = DEFAULT_EXECUTABLE,
  ): Promise<RustSandboxBackend> {
    try {
      const capability = await new Promise<SandboxCapability>(
        (resolveCapability, reject) => {
          const child = spawn(executable, ["__sandbox-probe"], {
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
          child.once("close", (code) => {
            if (code !== 0) {
              reject(new Error(stderr.trim() || `probe exited with ${String(code)}`));
              return;
            }
            try {
              resolveCapability(JSON.parse(stdout) as SandboxCapability);
            } catch (error) {
              reject(error);
            }
          });
        },
      );
      return new RustSandboxBackend(capability, executable);
    } catch (error) {
      return new RustSandboxBackend(
        disabledSandbox(
          error instanceof Error ? error.message : String(error),
        ),
        executable,
      );
    }
  }

  status(): SandboxStatus {
    const enforced = this.capability.mode === "enforced";
    return {
      mode: this.capability.mode,
      backend: this.capability.backend,
      filesystem: enforced ? "workspace-write" : "full-access",
      network: enforced ? "blocked" : "allowed",
      ...(this.capability.reason ? { reason: this.capability.reason } : {}),
    };
  }

  operationsFor(profile: SandboxExecutionProfile): BashOperations {
    if (profile.mode !== "enforced" || profile.backend !== "native") {
      // ponytail: degraded path reuses Pi's own local shell; sandbox enforcement
      // is replaced by explicit approval, add a real fallback backend if needed.
      return createLocalBashOperations();
    }

    return {
      exec: async (command, cwd, { onData, signal, timeout, env }) => {
        const request = {
          version: 1,
          cwd,
          command,
          ...(timeout === undefined ? {} : { timeoutMs: timeout * 1000 }),
          profile: {
            filesystem: {
              readOnly: [],
              readWrite: profile.filesystem.readWrite,
              denyRead: profile.filesystem.denyRead,
              denyWrite: profile.filesystem.denyWrite,
            },
            network: profile.network === "allowed" ? "allow" : "deny",
            protectedPaths: profile.filesystem.denyRead,
          },
          environment: env ?? {},
        };
        const child = spawn(
          this.executable,
          ["__sandbox-exec"],
          {
            cwd,
            env: { ...process.env, ...env },
            stdio: ["pipe", "pipe", "pipe"],
            detached: process.platform !== "win32",
          },
        );
        child.stdin.end(JSON.stringify(request));

        let timedOut = false;
        let aborted = false;
        const killTree = () => {
          if (child.pid === undefined) return;
          if (process.platform === "win32") {
            child.kill("SIGKILL");
          } else {
            try {
              process.kill(-child.pid, "SIGKILL");
            } catch {
              child.kill("SIGKILL");
            }
          }
        };
        const onAbort = () => {
          aborted = true;
          killTree();
        };
        signal?.addEventListener("abort", onAbort, { once: true });
        const timeoutHandle =
          timeout === undefined
            ? undefined
            : setTimeout(() => {
                timedOut = true;
                killTree();
              }, timeout * 1000);

        child.stdout.on("data", onData);
        child.stderr.on("data", onData);

        try {
          const exitCode = await new Promise<number | null>(
            (resolveExit, reject) => {
              child.once("error", reject);
              child.once("close", (code) => resolveExit(code));
            },
          );
          if (aborted || signal?.aborted) throw new Error("aborted");
          if (timedOut) throw new Error(`timeout:${String(timeout)}`);
          if (exitCode === 124) throw new Error(`timeout:${String(timeout)}`);
          return { exitCode };
        } finally {
          signal?.removeEventListener("abort", onAbort);
          if (timeoutHandle !== undefined) clearTimeout(timeoutHandle);
        }
      },
    };
  }
}

export type { ChildProcessWithoutNullStreams };
