import type { RuntimeSupervisor } from "../../runtime/runtime-supervisor.ts";
import type { PlanModeService } from "../../runtime/plan-mode-service.ts";
import type { JsonObject } from "../../protocol/validation.ts";

export class SessionService {
  private readonly runtime: RuntimeSupervisor;
  private readonly planMode: PlanModeService;
  private readonly onTransition: () => void;
  private readonly activation: () => JsonObject;

  constructor(
    runtime: RuntimeSupervisor,
    planMode: PlanModeService,
    onTransition: () => void,
    activation: () => JsonObject,
  ) {
    this.runtime = runtime;
    this.planMode = planMode;
    this.onTransition = onTransition;
    this.activation = activation;
  }

  async newSession(): Promise<{ cancelled: boolean; activation?: JsonObject }> {
    const runtime = this.runtime.requireIdle("Cannot create a session");
    if (this.planMode.current()) {
      this.planMode.set(runtime.session, false);
    }
    const result = await this.runtime.newSession();
    if (result.cancelled) return { cancelled: true };
    this.onTransition();
    return { cancelled: false, activation: this.activation() };
  }

  async resumeSession(input: {
    sessionPath: string;
    cwdOverride?: string;
  }): Promise<{ cancelled: boolean; activation?: JsonObject }> {
    this.runtime.requireIdle("Cannot resume a session");
    const result = await this.runtime.switchSession(input.sessionPath, {
      ...(input.cwdOverride ? { cwdOverride: input.cwdOverride } : {}),
    });
    if (result.cancelled) return { cancelled: true };
    this.onTransition();
    return { cancelled: false, activation: this.activation() };
  }

  clearQueue(): JsonObject {
    const queue = this.runtime.current().session.clearQueue();
    return {
      ...queue,
      restoredText: [...queue.steering, ...queue.followUp].join("\n\n"),
    };
  }
}
