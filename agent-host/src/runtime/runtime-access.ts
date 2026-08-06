import type { AgentSessionRuntime } from "@earendil-works/pi-coding-agent";

export interface RuntimeAccess {
  current(): AgentSessionRuntime;
  requireIdle(operation: string): AgentSessionRuntime;
  sessionGeneration(): number;
}
