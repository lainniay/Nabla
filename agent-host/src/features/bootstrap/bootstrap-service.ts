import type { ContextSnapshot } from "../context/model.ts";
import type { ResourceSnapshot } from "../workspace/config.ts";
import type { PlanArtifact } from "../plans/model.ts";
import type {
  AgentsSnapshot,
  BootstrapState,
  PendingIntegrationSnapshot,
  SandboxStatus,
} from "../../protocol/contracts.ts";

export interface BootstrapInput {
  scopeId: string;
  planMode: { active: boolean; activeTools: string[] };
  artifact: PlanArtifact | null;
  resources: ResourceSnapshot;
  agents: AgentsSnapshot;
  context: ContextSnapshot;
  sandbox: SandboxStatus;
  pendingIntegrations: PendingIntegrationSnapshot[];
  warnings: string[];
}

export class BootstrapService {
  private readonly read: () => BootstrapInput;

  constructor(read: () => BootstrapInput) {
    this.read = read;
  }

  snapshot(): BootstrapState {
    const input = this.read();
    return {
      scopeId: input.scopeId,
      planMode: input.planMode,
      plan: { artifact: input.artifact },
      resources: input.resources,
      agents: input.agents,
      context: input.context,
      sandbox: input.sandbox,
      pendingIntegrations: input.pendingIntegrations,
      warnings: input.warnings,
    };
  }
}
