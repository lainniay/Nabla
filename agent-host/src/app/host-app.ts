import type {
  AgentSessionRuntime,
  SessionManager,
} from "@earendil-works/pi-coding-agent";

import { isJsonObject, type JsonObject } from "../protocol/validation.ts";
import type { ControlServer } from "../transport/control-server.ts";
import type { RuntimeSupervisor } from "../runtime/runtime-supervisor.ts";
import type { IntegrationService } from "../features/subagents/isolation/integration-service.ts";
import type { SubagentSupervisor } from "../features/subagents/subagent-supervisor.ts";
import type { AuthService } from "../features/auth/auth-service.ts";
import type { InteractionBroker } from "../features/interactions/interaction-broker.ts";
import type { ActiveSubagent } from "../features/subagents/subagent-types.ts";

export interface HostApp {
  runtime(): AgentSessionRuntime;
  start(): Promise<void>;
  close(): Promise<void>;
}

export class HostAppImpl implements HostApp {
  private readonly supervisor: RuntimeSupervisor;
  private readonly control: ControlServer;
  private readonly integrations: IntegrationService;
  private readonly subagents: SubagentSupervisor;
  private readonly auth: AuthService;
  private readonly interactions: InteractionBroker;
  private readonly startupSessionManager: SessionManager;
  private readonly cwd: string;
  private readonly agentDir: string;

  constructor(
    supervisor: RuntimeSupervisor,
    control: ControlServer,
    integrations: IntegrationService,
    subagents: SubagentSupervisor,
    auth: AuthService,
    interactions: InteractionBroker,
    startupSessionManager: SessionManager,
    cwd: string,
    agentDir: string,
  ) {
    this.supervisor = supervisor;
    this.control = control;
    this.integrations = integrations;
    this.subagents = subagents;
    this.auth = auth;
    this.interactions = interactions;
    this.startupSessionManager = startupSessionManager;
    this.cwd = cwd;
    this.agentDir = agentDir;
  }

  runtime(): AgentSessionRuntime {
    return this.supervisor.current();
  }

  async start(): Promise<void> {
    if (!this.supervisor.hasRuntime()) {
      await this.supervisor.initialize({
        cwd: this.cwd,
        agentDir: this.agentDir,
        sessionManager: this.startupSessionManager,
      });
    }
    await this.recoverWorktrees();
    await this.control.listen();
  }

  async close(): Promise<void> {
    this.auth.cancel("Authentication host stopped");
    this.interactions.cancelAll();
    await this.subagents.hostClose();
    await this.control.close();
    await this.supervisor.close();
  }

  private async recoverWorktrees(): Promise<void> {
    const runtime = this.supervisor.current();
    const cwd = runtime.session.sessionManager.getCwd();
    const recovered = await this.integrations.recover(cwd);
    for (const { record, metadata, profile } of recovered) {
      const result: JsonObject =
        metadata.result && isJsonObject(metadata.result)
          ? metadata.result
          : {
              status: "blocked",
              summary:
                "Recovered isolated subagent changes after the host restarted",
              evidence: [],
              changedPaths: record.changedPaths,
              verification: [],
              blockers: ["Integration was interrupted before completion"],
            };
      const active: ActiveSubagent = {
        id: record.agentId,
        profile: metadata.profile,
        task: metadata.task,
        direct: metadata.direct,
        planReadOnly: metadata.planReadOnly,
        lifecycle: "awaiting_integration",
        originSession: runtime.session,
        originSessionId: metadata.originSessionId,
        controller: new AbortController(),
        startedAt: record.createdAt,
        turns: 0,
        maxTurns: profile.maxTurns,
        model: metadata.model,
        isolationBackend: "worktree",
        integrationStatus: record.integrationStatus,
        worktree: record,
      };
      this.subagents.restoreRecovered(active, result, record);
    }
  }
}
