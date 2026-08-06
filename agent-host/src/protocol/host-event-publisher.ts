import type { HostEvent } from "./contracts.ts";

export type EventDelivery = "reliable" | "coalescible";

export type OutboundHostEvent = HostEvent & { scopeId?: string };

export interface EventSink {
  (event: OutboundHostEvent): void;
}

export class HostEventPublisher {
  private scopeIdProvider: () => string | undefined = () => undefined;
  private readonly sink: EventSink;

  constructor(sink: EventSink) {
    this.sink = sink;
  }

  setScopeIdProvider(provider: () => string | undefined): void {
    this.scopeIdProvider = provider;
  }

  publish(
    event: HostEvent,
    options: { scopeId?: string; delivery?: EventDelivery } = {},
  ): void {
    const existingScopeId =
      "scopeId" in event && typeof event.scopeId === "string"
        ? event.scopeId
        : undefined;
    const scopeId = existingScopeId ?? options.scopeId ?? this.scopeIdProvider();
    this.sink(scopeId ? { ...event, scopeId } : event);
  }
}
