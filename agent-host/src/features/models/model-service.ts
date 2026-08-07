import type { ModelRuntime } from "@earendil-works/pi-coding-agent";

import type { ThinkingLevel } from "../permissions/shell/rules.ts";
import type { RuntimeAccess } from "../../runtime/runtime-access.ts";
import type { JsonObject } from "../../protocol/validation.ts";

export interface ModelListSnapshot {
  current: { provider: string; id: string } | null;
  models: Array<{
    provider: string;
    id: string;
    name: string;
    reasoning: unknown;
    contextWindow: unknown;
  }>;
}

export class ModelService {
  private readonly modelRuntime: ModelRuntime;
  private readonly runtime: RuntimeAccess;

  constructor(
    modelRuntime: ModelRuntime,
    runtime: RuntimeAccess,
  ) {
    this.modelRuntime = modelRuntime;
    this.runtime = runtime;
  }

  async list(): Promise<ModelListSnapshot> {
    const runtime = this.runtime.current();
    const models = await this.modelRuntime.getAvailable();
    return {
      current: runtime.session.model
        ? {
            provider: runtime.session.model.provider,
            id: runtime.session.model.id,
          }
        : null,
      models: models.map((model) => ({
        provider: model.provider,
        id: model.id,
        name: model.name,
        reasoning: model.reasoning,
        contextWindow: model.contextWindow,
      })),
    };
  }

  async set(input: {
    provider: string;
    modelId: string;
  }): Promise<{ provider: string; id: string; name: string }> {
    const runtime = this.runtime.requireIdle("Cannot change model");
    const model = this.modelRuntime.getModel(input.provider, input.modelId);
    if (!model) {
      throw new Error(`Unknown model: ${input.provider}/${input.modelId}`);
    }
    await runtime.session.setModel(model);
    return {
      provider: input.provider,
      id: input.modelId,
      name: model.name,
    };
  }

  setThinking(level: ThinkingLevel): JsonObject {
    const runtime = this.runtime.requireIdle("Cannot change thinking level");
    runtime.session.setThinkingLevel(level);
    return {
      level: runtime.session.thinkingLevel,
      available: runtime.session.getAvailableThinkingLevels(),
    };
  }

  async selectDefaultModel(providerId: string): Promise<unknown> {
    const runtime = this.runtime.current();
    try {
      if (runtime.session.model) return runtime.session.model;
      const available = await this.modelRuntime.getAvailable(providerId);
      if (available.length === 0) return undefined;
      const settings = runtime.services.settingsManager;
      const defaultModel =
        settings.getDefaultProvider() === providerId
          ? available.find(
              (model) => model.id === settings.getDefaultModel(),
            )
          : undefined;
      const selectedModel = defaultModel ?? available[0];
      await runtime.session.setModel(selectedModel);
      return selectedModel;
    } catch {
      return undefined;
    }
  }
}
