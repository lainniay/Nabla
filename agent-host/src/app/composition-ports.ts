import type { ActiveAgentSnapshot } from "../protocol/contracts.ts";
import type {
  EventSink,
  OutboundHostEvent,
} from "../protocol/host-event-publisher.ts";
import type { ControlServer } from "../transport/control-server.ts";

export class EventRoute {
  private deliver: EventSink = () => undefined;

  readonly sink: EventSink = (event: OutboundHostEvent) => {
    this.deliver(event);
  };

  bind(deliver: EventSink): void {
    this.deliver = deliver;
  }
}

export class ConnectionState {
  private control: ControlServer | undefined;

  bind(control: ControlServer): void {
    this.control = control;
  }

  hasConnection(): boolean {
    return this.control?.hasConnection() ?? false;
  }

  isCurrent(context: Parameters<ControlServer["isCurrent"]>[0]): boolean {
    return this.control?.isCurrent(context) ?? false;
  }
}

export class SubagentStateSource {
  private read: () => {
    active: ActiveAgentSnapshot[];
    pending: ActiveAgentSnapshot[];
  } = () => ({ active: [], pending: [] });

  bind(source: {
    activeSnapshots(): ActiveAgentSnapshot[];
    pendingSnapshots(): ActiveAgentSnapshot[];
  }): void {
    this.read = () => ({
      active: source.activeSnapshots(),
      pending: source.pendingSnapshots(),
    });
  }

  snapshot(): {
    active: ActiveAgentSnapshot[];
    pending: ActiveAgentSnapshot[];
  } {
    return this.read();
  }
}
