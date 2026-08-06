import type {
  AgentSessionRuntime,
  SessionManager,
} from "@earendil-works/pi-coding-agent";

import type { HostBridge } from "../legacy-host-bridge.ts";
import type { RuntimeSupervisor } from "../runtime/runtime-supervisor.ts";

export interface HostApp {
  runtime(): AgentSessionRuntime;
  start(): Promise<void>;
  close(): Promise<void>;
}

export class HostAppImpl implements HostApp {
  private readonly supervisor: RuntimeSupervisor;
  private readonly bridge: HostBridge;
  private readonly startupSessionManager: SessionManager;
  private readonly cwd: string;
  private readonly agentDir: string;

  constructor(
    supervisor: RuntimeSupervisor,
    bridge: HostBridge,
    startupSessionManager: SessionManager,
    cwd: string,
    agentDir: string,
  ) {
    this.supervisor = supervisor;
    this.bridge = bridge;
    this.startupSessionManager = startupSessionManager;
    this.cwd = cwd;
    this.agentDir = agentDir;
  }

  runtime(): AgentSessionRuntime {
    return this.supervisor.current();
  }

  async start(): Promise<void> {
    await this.supervisor.initialize({
      cwd: this.cwd,
      agentDir: this.agentDir,
      sessionManager: this.startupSessionManager,
    });
    await this.bridge.listen();
  }

  async close(): Promise<void> {
    await this.bridge.close();
    await this.supervisor.close();
  }
}
