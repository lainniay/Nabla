import type {
  AgentSession,
  AgentSessionRuntime,
  ModelRuntime,
} from "@earendil-works/pi-coding-agent";

import {
  loadHarnessConfig,
  modelReference,
  saveWorkspaceTrust,
  workspaceIsTrusted,
  type AgentProfile,
  type HarnessConfig,
  type ResourceSnapshot,
} from "../../harness.ts";
import type { RuntimeAccess } from "../../runtime/runtime-access.ts";
import type { PlanModeService } from "../../runtime/plan-mode-service.ts";
import type { AgentsSnapshot } from "../../protocol/contracts.ts";
import type { JsonObject } from "../../protocol/validation.ts";

export class WorkspaceService {
  private readonly runtime: RuntimeAccess;
  private readonly planMode: PlanModeService;
  private readonly modelRuntime: ModelRuntime;
  private readonly send: (event: JsonObject) => void;
  private config: HarnessConfig;
  private resourceRevision = 1;

  constructor(
    runtime: RuntimeAccess,
    planMode: PlanModeService,
    modelRuntime: ModelRuntime,
    send: (event: JsonObject) => void,
    initialConfig: HarnessConfig,
  ) {
    this.runtime = runtime;
    this.planMode = planMode;
    this.modelRuntime = modelRuntime;
    this.send = send;
    this.config = initialConfig;
  }

  configValue(): HarnessConfig {
    return this.config;
  }

  reloadConfig(cwd: string): void {
    this.config = loadHarnessConfig(cwd);
  }

  activate(
    cwd: string,
    session: AgentSession,
    agents: () => AgentsSnapshot,
  ): void {
    this.config = loadHarnessConfig(cwd);
    this.publishWorkspaceState(session, agents());
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

  async reloadResources(
    agents: () => AgentsSnapshot,
  ): Promise<ResourceSnapshot> {
    const runtime = this.runtime.requireIdle("Cannot reload resources");
    this.config = loadHarnessConfig(
      runtime.session.sessionManager.getCwd(),
    );
    await runtime.session.reload();
    this.planMode.apply(runtime.session);
    this.sendPlanModeState(runtime);
    const { resources } = this.publishWorkspaceState(
      runtime.session,
      agents(),
    );
    return resources;
  }

  async setWorkspaceTrust(
    trusted: boolean,
    agents: () => AgentsSnapshot,
  ): Promise<ResourceSnapshot> {
    const runtime = this.runtime.requireIdle("Cannot change workspace trust");
    const cwd = runtime.session.sessionManager.getCwd();
    this.config = saveWorkspaceTrust(cwd, trusted);
    this.config = loadHarnessConfig(cwd);
    runtime.services.settingsManager.setProjectTrusted(trusted);
    await runtime.session.resourceLoader.reload({
      resolveProjectTrust: async () => trusted,
    });
    await runtime.session.reload();
    this.planMode.apply(runtime.session);
    this.sendPlanModeState(runtime);
    const { resources } = this.publishWorkspaceState(
      runtime.session,
      agents(),
    );
    return resources;
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

  private sendPlanModeState(runtime: AgentSessionRuntime): void {
    this.send({
      type: "plan_mode_state",
      active: this.planMode.current(),
      activeTools: runtime.session.getActiveToolNames(),
    });
  }
}
