import {
  createAgentSessionRuntime,
  type AgentSessionRuntime,
  type CreateAgentSessionRuntimeFactory,
  type SessionManager,
} from "@earendil-works/pi-coding-agent";

import type { SessionRuntimePort } from "./runtime-access.ts";

export interface SessionTransition {
  cancelled: boolean;
}

export interface NewSessionOptions {
  parentSession?: string;
  setup?: (sessionManager: SessionManager) => Promise<void>;
}

export interface SwitchSessionOptions {
  cwdOverride?: string;
}

export interface InitializeOptions {
  cwd: string;
  agentDir: string;
  sessionManager: SessionManager;
}

export class RuntimeSupervisor implements SessionRuntimePort {
  private readonly factory: CreateAgentSessionRuntimeFactory;
  private runtime?: AgentSessionRuntime;
  private generation = 0;

  constructor(
    factory: CreateAgentSessionRuntimeFactory,
    initialRuntime?: AgentSessionRuntime,
  ) {
    this.factory = factory;
    if (initialRuntime) {
      this.runtime = initialRuntime;
      this.generation = 1;
    }
  }

  current(): AgentSessionRuntime {
    if (!this.runtime) throw new Error("Agent runtime is not ready");
    return this.runtime;
  }

  requireIdle(operation: string): AgentSessionRuntime {
    const runtime = this.current();
    if (!runtime.session.isIdle) {
      throw new Error(`${operation} while the agent is running`);
    }
    return runtime;
  }

  sessionGeneration(): number {
    return this.generation;
  }

  hasRuntime(): boolean {
    return this.runtime !== undefined;
  }

  async initialize(options: InitializeOptions): Promise<AgentSessionRuntime> {
    this.runtime = await createAgentSessionRuntime(this.factory, options);
    this.generation += 1;
    return this.runtime;
  }

  async newSession(options: NewSessionOptions = {}): Promise<SessionTransition> {
    const runtime = this.current();
    const result = await runtime.newSession(options);
    if (!result.cancelled) this.generation += 1;
    return result;
  }

  async switchSession(
    sessionPath: string,
    options: SwitchSessionOptions = {},
  ): Promise<SessionTransition> {
    const runtime = this.current();
    const result = await runtime.switchSession(sessionPath, {
      ...(options.cwdOverride ? { cwdOverride: options.cwdOverride } : {}),
    });
    if (!result.cancelled) this.generation += 1;
    return result;
  }

  async close(): Promise<void> {
    const runtime = this.runtime;
    this.runtime = undefined;
    if (runtime) await runtime.dispose();
  }
}
