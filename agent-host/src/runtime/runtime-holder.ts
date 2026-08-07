import type { SessionRuntimePort } from "./runtime-access.ts";

/**
 * Late-bound runtime access for the composition root: modules are assembled
 * before the RuntimeSupervisor exists, then bound once it is created.
 */
export class RuntimeHolder implements SessionRuntimePort {
  private runtime: SessionRuntimePort | undefined;

  bind(runtime: SessionRuntimePort): void {
    this.runtime = runtime;
  }

  current() {
    return this.require().current();
  }

  requireIdle(operation: string) {
    return this.require().requireIdle(operation);
  }

  sessionGeneration(): number {
    return this.require().sessionGeneration();
  }

  newSession(options?: Parameters<SessionRuntimePort["newSession"]>[0]) {
    return this.require().newSession(options);
  }

  switchSession(
    sessionPath: string,
    options?: Parameters<SessionRuntimePort["switchSession"]>[1],
  ) {
    return this.require().switchSession(sessionPath, options);
  }

  private require(): SessionRuntimePort {
    if (!this.runtime) throw new Error("Agent runtime is not ready");
    return this.runtime;
  }
}
