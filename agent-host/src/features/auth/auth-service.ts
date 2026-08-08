import type { ModelRuntime } from "@earendil-works/pi-coding-agent";

import { AuthPromptQueue } from "../../auth-prompts.ts";
import { asError, type JsonObject } from "../../protocol/validation.ts";

type AuthInteraction = Parameters<ModelRuntime["login"]>[2];
type AuthPrompt = Parameters<AuthInteraction["prompt"]>[0];
type AuthEvent = Parameters<AuthInteraction["notify"]>[0];

interface ActiveFlow {
  id: string;
  controller: AbortController;
  prompts: AuthPromptQueue;
  nextPromptId: number;
}

export class AuthService {
  private readonly modelRuntime: ModelRuntime;
  private readonly afterLogin: (providerId: string) => Promise<unknown>;
  private readonly send: (event: JsonObject) => void;
  private activeFlow?: ActiveFlow;

  constructor(
    modelRuntime: ModelRuntime,
    afterLogin: (providerId: string) => Promise<unknown>,
    send: (event: JsonObject) => void,
  ) {
    this.modelRuntime = modelRuntime;
    this.afterLogin = afterLogin;
    this.send = send;
  }

  async listProviders(): Promise<unknown[]> {
    const providers = await Promise.all(
      this.modelRuntime.getProviders().map(async (provider) => {
        const status = await this.modelRuntime.checkAuth(provider.id);
        const methods: JsonObject[] = [];
        if (provider.auth.oauth) {
          methods.push({
            type: "oauth",
            label:
              provider.auth.oauth.loginLabel ??
              provider.auth.oauth.name ??
              "Sign in with an account",
            available: true,
          });
        }
        if (provider.auth.apiKey) {
          methods.push({
            type: "api_key",
            label: provider.auth.apiKey.name ?? "API key",
            available: typeof provider.auth.apiKey.login === "function",
          });
        }
        return {
          id: provider.id,
          name: provider.name,
          configured: status !== undefined,
          configuredType: status?.type,
          configuredSource: status?.source,
          methods,
        };
      }),
    );
    return providers
      .filter((provider) =>
        (provider.methods as JsonObject[]).some(
          (method) => method.available === true,
        ),
      )
      .sort((left, right) => left.name.localeCompare(right.name));
  }

  startLogin(input: {
    flowId: string;
    providerId: string;
    authType: "oauth" | "api_key";
  }): Promise<{
    providerId: string;
    credentialType: string;
    selectedModel: unknown;
  }> {
    if (this.activeFlow) throw new Error("Another login flow is already active");

    const { flowId, providerId, authType } = input;
    const provider = this.modelRuntime.getProvider(providerId);
    if (!provider) throw new Error(`Unknown provider: ${providerId}`);
    if (authType === "oauth" && !provider.auth.oauth) {
      throw new Error(`${provider.name} does not support OAuth login`);
    }
    if (authType === "api_key" && !provider.auth.apiKey?.login) {
      throw new Error(`${provider.name} does not support in-app API key login`);
    }

    const flow: ActiveFlow = {
      id: flowId,
      controller: new AbortController(),
      prompts: new AuthPromptQueue(),
      nextPromptId: 1,
    };
    this.activeFlow = flow;

    return new Promise((resolve, reject) => {
      void this.modelRuntime
        .login(providerId, authType, {
          signal: flow.controller.signal,
          prompt: (prompt) => this.prompt(flow, prompt),
          notify: (event) => this.notify(flow, event),
        })
        .then(async (credential) => {
          const selectedModel = await this.afterLogin(providerId);
          this.send({
            type: "auth_complete",
            flowId,
            providerId,
            credentialType: credential.type,
            selectedModel,
          });
          resolve({
            providerId,
            credentialType: credential.type,
            selectedModel,
          });
        })
        .catch((error) => {
          reject(asError(error));
        })
        .finally(() => {
          if (this.activeFlow === flow) this.activeFlow = undefined;
          this.rejectPrompts(flow, "Login flow ended");
        });
    });
  }

  replyToPrompt(input: {
    flowId: string;
    promptId: string;
    value: string;
  }): void {
    const flow = this.activeFlow;
    if (!flow || flow.id !== input.flowId) {
      throw new Error("Login flow is no longer active");
    }
    if (!flow.prompts.reply(input.promptId, input.value)) {
      throw new Error("Authentication prompt is no longer active");
    }
  }

  cancel(reason: string): void {
    const flow = this.activeFlow;
    if (!flow) return;
    flow.controller.abort();
    this.rejectPrompts(flow, reason);
  }

  cancelLogin(): void {
    this.cancel("Login cancelled");
  }

  async logout(providerId: string): Promise<void> {
    await this.modelRuntime.logout(providerId);
  }

  private prompt(flow: ActiveFlow, prompt: AuthPrompt): Promise<string> {
    const promptId = String(flow.nextPromptId++);
    return flow.prompts.request(
      promptId,
      [prompt.signal, flow.controller.signal],
      () =>
        this.send({
          type: "auth_prompt",
          flowId: flow.id,
          promptId,
          promptType: prompt.type,
          message: prompt.message,
          placeholder: "placeholder" in prompt ? prompt.placeholder : undefined,
          options: prompt.type === "select" ? prompt.options : undefined,
        }),
      () =>
        this.send({
          type: "auth_prompt_cancelled",
          flowId: flow.id,
          promptId,
        }),
    );
  }

  private notify(flow: ActiveFlow, event: AuthEvent): void {
    this.send({
      type: "auth_notify",
      flowId: flow.id,
      event,
    });
  }

  private rejectPrompts(flow: ActiveFlow, reason: string): void {
    flow.prompts.cancelAll(reason);
  }
}
