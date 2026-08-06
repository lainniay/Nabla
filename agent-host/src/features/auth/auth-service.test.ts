import assert from "node:assert/strict";
import test from "node:test";

import type { ModelRuntime } from "@earendil-works/pi-coding-agent";

import type { JsonObject } from "../../protocol/validation.ts";
import { AuthService } from "./auth-service.ts";

function fakeModelRuntime(
  overrides: Partial<ModelRuntime> = {},
): ModelRuntime {
  return {
    getProviders: () => [
      {
        id: "fake",
        name: "Fake",
        auth: {
          apiKey: { name: "API key", login: async () => undefined },
          oauth: { name: "OAuth", login: async () => undefined },
        },
      },
    ],
    checkAuth: async () => undefined,
    getProvider: (providerId: string) => ({
      id: providerId,
      name: "Fake",
      auth: {
        apiKey: { name: "API key", login: async () => undefined },
        oauth: { name: "OAuth", login: async () => undefined },
      },
    }),
    login: async () => ({ type: "api_key" }),
    logout: async () => undefined,
    ...overrides,
  } as unknown as ModelRuntime;
}

function pendingLogin(): ModelRuntime["login"] {
  return async (
    _providerId: string,
    _authType: unknown,
    authInteraction: Parameters<ModelRuntime["login"]>[2],
  ) =>
    new Promise<never>((_resolve, reject) => {
      if (authInteraction.signal?.aborted) {
        reject(new Error("Login cancelled"));
        return;
      }
      authInteraction.signal?.addEventListener(
        "abort",
        () => reject(new Error("Login cancelled")),
        { once: true },
      );
    });
}

test("listProviders returns available methods without credentials", async () => {
  const events: JsonObject[] = [];
  const service = new AuthService(
    fakeModelRuntime(),
    async () => undefined,
    (event) => events.push(event),
  );
  const providers = (await service.listProviders()) as Array<{
    id: string;
    methods: Array<{ type: string }>;
  }>;
  assert.equal(providers.length, 1);
  assert.deepEqual(
    providers[0]?.methods.map((method) => method.type).sort(),
    ["api_key", "oauth"],
  );
});

test("startLogin resolves credentials and emits auth_complete", async () => {
  const events: JsonObject[] = [];
  let selected = 0;
  const service = new AuthService(
    fakeModelRuntime(),
    async () => {
      selected += 1;
      return { id: "default" };
    },
    (event) => events.push(event),
  );
  const result = await service.startLogin({
    flowId: "flow-1",
    providerId: "fake",
    authType: "api_key",
  });
  assert.equal(result.credentialType, "api_key");
  assert.deepEqual(result.selectedModel, { id: "default" });
  assert.equal(selected, 1);
  assert.ok(events.some((event) => event.type === "auth_complete"));
});

test("prompts are announced and replied in order", async () => {
  const events: JsonObject[] = [];
  let interaction!: Parameters<ModelRuntime["login"]>[2];
  const service = new AuthService(
    fakeModelRuntime({
      login: async (...args) => {
        const authInteraction = args[2] as Parameters<ModelRuntime["login"]>[2];
        interaction = authInteraction;
        return new Promise<never>((_resolve, reject) => {
          if (authInteraction.signal?.aborted) {
            reject(new Error("Login cancelled"));
            return;
          }
          authInteraction.signal?.addEventListener(
            "abort",
            () => reject(new Error("Login cancelled")),
            { once: true },
          );
        });
      },
    }),
    async () => undefined,
    (event) => events.push(event),
  );
  const login = service.startLogin({
    flowId: "flow-1",
    providerId: "fake",
    authType: "api_key",
  });
  const prompt = interaction.prompt({ type: "text", message: "Key" } as never);
  assert.ok(events.some((event) => event.type === "auth_prompt"));
  service.replyToPrompt({ flowId: "flow-1", promptId: "1", value: "secret" });
  assert.equal(await prompt, "secret");
  assert.throws(
    () =>
      service.replyToPrompt({
        flowId: "flow-1",
        promptId: "1",
        value: "again",
      }),
    /Authentication prompt is no longer active/u,
  );
  login.catch(() => undefined);
  service.cancel("Login cancelled");
});

test("duplicate active logins are rejected", async () => {
  const service = new AuthService(
    fakeModelRuntime({
      login: pendingLogin(),
    }),
    async () => undefined,
    () => {},
  );
  const first = service.startLogin({
    flowId: "flow-1",
    providerId: "fake",
    authType: "api_key",
  });
  assert.throws(
    () =>
      service.startLogin({
        flowId: "flow-2",
        providerId: "fake",
        authType: "api_key",
      }),
    /Another login flow is already active/u,
  );
  service.cancel("Login cancelled");
  await assert.rejects(first, /Login cancelled/u);
});

test("cancel aborts the active flow and pending prompts", async () => {
  const events: JsonObject[] = [];
  let interaction!: Parameters<ModelRuntime["login"]>[2];
  const service = new AuthService(
    fakeModelRuntime({
      login: async (...args) => {
        const authInteraction = args[2] as Parameters<ModelRuntime["login"]>[2];
        interaction = authInteraction;
        return new Promise<never>((_resolve, reject) => {
          if (authInteraction.signal?.aborted) {
            reject(new Error("Login cancelled"));
            return;
          }
          authInteraction.signal?.addEventListener(
            "abort",
            () => reject(new Error("Login cancelled")),
            { once: true },
          );
        });
      },
    }),
    async () => undefined,
    (event) => events.push(event),
  );
  const login = service.startLogin({
    flowId: "flow-1",
    providerId: "fake",
    authType: "api_key",
  });
  const prompt = interaction.prompt({ type: "text", message: "Key" } as never);
  service.cancel("Host control client disconnected");
  await assert.rejects(login, /Login cancelled/u);
  await assert.rejects(prompt, /Login cancelled/u);
  assert.ok(events.some((event) => event.type === "auth_prompt_cancelled"));
});

test("logout delegates to the model runtime", async () => {
  let loggedOut = "";
  const service = new AuthService(
    fakeModelRuntime({
      logout: async (providerId: string) => {
        loggedOut = providerId;
      },
    }),
    async () => undefined,
    () => {},
  );
  await service.logout("fake");
  assert.equal(loggedOut, "fake");
});
