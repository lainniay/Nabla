import type {
  AgentSession,
  ModelRuntime,
} from "@earendil-works/pi-coding-agent";

import {
  loadHarnessConfig,
  type HarnessConfig,
  type ResourceSnapshot,
} from "./config.ts";
import {
  saveWorkspaceTrust,
  workspaceIsTrusted,
} from "./trust.ts";
import {
  modelReference,
  type AgentProfile,
} from "../subagents/profile-model.ts";
import { evaluateProfileToolExposure } from "../permissions/policy/profile-compiler.ts";
import type { RuntimeAccess } from "../../runtime/runtime-access.ts";
import type {
  ActiveAgentSnapshot,
  AgentsSnapshot,
} from "../../protocol/contracts.ts";
import type { JsonObject } from "../../protocol/validation.ts";

export class WorkspaceService {
  private readonly runtime: RuntimeAccess;
  private readonly modelRuntime: ModelRuntime;
  private readonly send: (event: JsonObject) => void;
  private readonly onReload: ((session: AgentSession) => void) | undefined;
  private readonly agents: () => {
    active: ActiveAgentSnapshot[];
    pending: ActiveAgentSnapshot[];
  };
  private readonly isConnected: () => boolean;
  private config: HarnessConfig;
  private resourceRevision = 1;
  private agentsRevision = 0;

  constructor(
    runtime: RuntimeAccess,
    modelRuntime: ModelRuntime,
    send: (event: JsonObject) => void,
    initialConfig: HarnessConfig,
    agents: () => { active: ActiveAgentSnapshot[]; pending: ActiveAgentSnapshot[] },
    isConnected: () => boolean,
    onReload?: (session: AgentSession) => void,
  ) {
    this.runtime = runtime;
    this.modelRuntime = modelRuntime;
    this.send = send;
    this.onReload = onReload;
    this.config = initialConfig;
    this.agents = agents;
    this.isConnected = isConnected;
  }

  configValue(): HarnessConfig {
    return this.config;
  }

  reloadConfig(cwd: string): void {
    this.config = loadHarnessConfig(cwd);
  }

  activate(cwd: string, session: AgentSession): void {
    this.config = loadHarnessConfig(cwd);
    if (this.isConnected()) {
      this.publishWorkspaceState(session, this.agentsSnapshot(session));
    }
  }

  resourceSnapshot(
    session = this.runtime.current().session,
  ): ResourceSnapshot {
    const loader = session.resourceLoader;
    const skills = loader.getSkills();
    const prompts = loader.getPrompts();
    const extensions = loader.getExtensions();
    return {
      scopeId: session.sessionId,
      trusted: workspaceIsTrusted(session.sessionManager.getCwd(), this.config),
      contextFiles: loader.getAgentsFiles().agentsFiles.map((file) => file.path),
      skills: skills.skills.map((skill) => ({
        name: skill.name,
        path: skill.filePath,
        description: skill.description,
      })),
      prompts: prompts.prompts.map((prompt) => ({
        name: prompt.name,
        path: prompt.filePath,
        description: prompt.description,
      })),
      extensions: extensions.extensions.map(
        (extension) => extension.resolvedPath,
      ),
      commands: [
        ...extensions.extensions.flatMap((extension) =>
          [...extension.commands.values()].map((command) => ({
            name: command.name,
            description: command.description ?? "",
            source: "extension" as const,
          })),
        ),
        ...prompts.prompts.map((prompt) => ({
          name: prompt.name,
          description: prompt.description,
          source: "prompt" as const,
        })),
        ...skills.skills.map((skill) => ({
          name: `skill:${skill.name}`,
          description: skill.description,
          source: "skill" as const,
        })),
      ],
      diagnostics: [
        ...skills.diagnostics,
        ...prompts.diagnostics,
        ...extensions.errors.map((error) => ({
          type: "error",
          message: error.error,
          path: error.path,
        })),
      ],
      revision: this.resourceRevision,
    };
  }

  async reloadResources(): Promise<ResourceSnapshot> {
    const runtime = this.runtime.requireIdle("Cannot reload resources");
    this.config = loadHarnessConfig(
      runtime.session.sessionManager.getCwd(),
    );
    await runtime.session.reload();
    this.onReload?.(runtime.session);
    const { resources } = this.publishWorkspaceState(
      runtime.session,
      this.agentsSnapshot(runtime.session),
    );
    return resources;
  }

  async setWorkspaceTrust(trusted: boolean): Promise<ResourceSnapshot> {
    const runtime = this.runtime.requireIdle("Cannot change workspace trust");
    const cwd = runtime.session.sessionManager.getCwd();
    this.config = saveWorkspaceTrust(cwd, trusted);
    this.config = loadHarnessConfig(cwd);
    runtime.services.settingsManager.setProjectTrusted(trusted);
    await runtime.session.resourceLoader.reload({
      resolveProjectTrust: async () => trusted,
    });
    await runtime.session.reload();
    this.onReload?.(runtime.session);
    const { resources } = this.publishWorkspaceState(
      runtime.session,
      this.agentsSnapshot(runtime.session),
    );
    return resources;
  }

  agentsSnapshot(session = this.runtime.current().session): AgentsSnapshot {
    return {
      scopeId: session.sessionId,
      revision: this.agentsRevision,
      maxParallel: this.config.maxParallel,
      profiles: Object.entries(this.config.profiles).map(([name, profile]) => ({
        unavailableReason:
          this.profileUnavailableReason(profile, session) ?? null,
        name,
        description: profile.description,
        source: profile.source,
        model: profile.model ?? null,
        thinkingLevel: profile.thinkingLevel ?? null,
        skills: profile.skills,
        tools: profile.tools,
        permission: profile.tools
          .map((tool) =>
            `${tool}:${evaluateProfileToolExposure(profile, tool)}`
          )
          .join(","),
        maxParallel: profile.maxParallel,
        maxTurns: profile.maxTurns,
        isolation: profile.isolation,
        disabled: profile.disabled,
      })),
      active: this.agents().active,
      pending: this.agents().pending,
      diagnostics: this.config.diagnostics,
    };
  }

  publishAgentsState(
    session = this.runtime.current().session,
  ): AgentsSnapshot {
    this.agentsRevision += 1;
    const snapshot = this.agentsSnapshot(session);
    this.send({ type: "agents_state", snapshot });
    return snapshot;
  }

  async reloadAgents(): Promise<AgentsSnapshot> {
    const runtime = this.runtime.current();
    this.reloadConfig(runtime.session.sessionManager.getCwd());
    return this.publishAgentsState(runtime.session);
  }

  profileUnavailableReason(
    profile: AgentProfile,
    session = this.runtime.current().session,
  ): string | undefined {
    const availableSkills = new Set(
      session.resourceLoader.getSkills().skills.map((skill) => skill.name),
    );
    const missingSkills = profile.skills.filter(
      (skill) => !availableSkills.has(skill),
    );
    if (missingSkills.length > 0) {
      return `Missing skills: ${missingSkills.join(", ")}`;
    }
    const reference = modelReference(profile);
    if (
      reference &&
      !this.modelRuntime.getModel(reference.provider, reference.id)
    ) {
      return `Configured model is unavailable: ${reference.provider}/${reference.id}`;
    }
    return undefined;
  }

  subagentCatalogPrompt(): string {
    const profiles = Object.entries(this.config.profiles)
      .filter(
        ([, profile]) =>
          !profile.disabled && !this.profileUnavailableReason(profile),
      )
      .map(
        ([name, profile]) =>
          `- ${name}: ${profile.description} (tools: ${profile.tools.join(", ") || "none"})`,
      );
    return profiles.length === 0
      ? "No subagent profiles are currently available."
      : [
          "Available subagents for delegate_task:",
          ...profiles,
          "Choose a profile only when its description matches the bounded task.",
        ].join("\n");
  }

  publishWorkspaceState(
    session: AgentSession,
    agents: AgentsSnapshot,
  ): { resources: ResourceSnapshot; agents: AgentsSnapshot } {
    this.resourceRevision += 1;
    const resources = this.resourceSnapshot(session);
    this.send({
      type: "workspace_state",
      scopeId: session.sessionId,
      resources,
      agents,
    });
    return { resources, agents };
  }

}
