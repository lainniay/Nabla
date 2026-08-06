import type { AgentSessionRuntime } from "@earendil-works/pi-coding-agent";

import type { HostEvent } from "../protocol/contracts.ts";

export interface HostEventPublisher {
  publish(
    event: HostEvent,
    options?: {
      scopeId?: string;
      delivery?: "reliable" | "coalescible";
    },
  ): void;
}

export interface RuntimeAccess {
  current(): AgentSessionRuntime;
  requireIdle(operation: string): AgentSessionRuntime;
  sessionGeneration(): number;
}

export interface HostDiagnosticsPort {
  warn(message: string, context?: Record<string, unknown>): void;
  snapshot(): readonly string[];
}
