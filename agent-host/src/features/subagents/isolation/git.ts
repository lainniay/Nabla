import { spawn } from "node:child_process";

import { DEFAULT_GIT_TIMEOUT_MS } from "./model.ts";

export interface GitResult {
  code: number;
  stdout: string;
  stderr: string;
}

export class GitClient {
  private readonly timeoutMs: number;

  constructor(timeoutMs: number = DEFAULT_GIT_TIMEOUT_MS) {
    this.timeoutMs = timeoutMs;
  }

  run(
    cwd: string,
    args: string[],
    options: {
      allowFailure?: boolean;
      env?: NodeJS.ProcessEnv;
      input?: string;
      signal?: AbortSignal;
    } = {},
  ): Promise<GitResult> {
    return new Promise((resolvePromise, reject) => {
      const child = spawn("git", ["-C", cwd, ...args], {
        cwd,
        env: { ...process.env, ...options.env },
        stdio: ["pipe", "pipe", "pipe"],
      });
      const stdout: Buffer[] = [];
      const stderr: Buffer[] = [];
      const timer = setTimeout(() => {
        child.kill("SIGTERM");
        reject(new Error(`Git command timed out: git ${args.join(" ")}`));
      }, this.timeoutMs);
      const abort = () => child.kill("SIGTERM");
      options.signal?.addEventListener("abort", abort, { once: true });
      child.stdout.on("data", (chunk: Buffer) => stdout.push(chunk));
      child.stderr.on("data", (chunk: Buffer) => stderr.push(chunk));
      child.on("error", (error) => {
        clearTimeout(timer);
        options.signal?.removeEventListener("abort", abort);
        reject(error);
      });
      child.on("close", (code) => {
        clearTimeout(timer);
        options.signal?.removeEventListener("abort", abort);
        const result = {
          code: code ?? 1,
          stdout: Buffer.concat(stdout).toString("utf8"),
          stderr: Buffer.concat(stderr).toString("utf8"),
        };
        if (result.code !== 0 && !options.allowFailure) {
          reject(
            new Error(
              result.stderr.trim() ||
                result.stdout.trim() ||
                `Git command failed (${result.code}): git ${args.join(" ")}`,
            ),
          );
        } else {
          resolvePromise(result);
        }
      });
      if (options.input) child.stdin.end(options.input);
      else child.stdin.end();
    });
  }
}
