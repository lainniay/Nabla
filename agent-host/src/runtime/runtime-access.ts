import type { AgentSessionRuntime } from "@earendil-works/pi-coding-agent";

import type {
  NewSessionOptions,
  SwitchSessionOptions,
} from "./runtime-supervisor.ts";

export interface RuntimeAccess {
  current(): AgentSessionRuntime;
  requireIdle(operation: string): AgentSessionRuntime;
  sessionGeneration(): number;
}

export interface SessionRuntimePort extends RuntimeAccess {
  newSession(options?: NewSessionOptions): Promise<{ cancelled: boolean }>;
  switchSession(
    sessionPath: string,
    options?: SwitchSessionOptions,
  ): Promise<{ cancelled: boolean }>;
}
