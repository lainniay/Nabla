import type { HostEvent } from "../protocol/contracts.ts";
export type { RuntimeAccess } from "../runtime/runtime-access.ts";

export interface HostEventPublisher {
  publish(
    event: HostEvent,
    options?: {
      scopeId?: string;
      delivery?: "reliable" | "coalescible";
    },
  ): void;
}

export interface HostDiagnosticsPort {
  warn(message: string, context?: Record<string, unknown>): void;
  snapshot(): readonly string[];
}
