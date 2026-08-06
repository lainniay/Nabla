import assert from "node:assert/strict";
import test from "node:test";

import type { SessionManager } from "@earendil-works/pi-coding-agent";

import { RuntimeSupervisor } from "../runtime/runtime-supervisor.ts";
import type { ControlServer } from "../transport/control-server.ts";
import type { IntegrationService } from "../features/subagents/integration-service.ts";
import type { SubagentSupervisor } from "../features/subagents/subagent-supervisor.ts";
import type { AuthService } from "../features/auth/auth-service.ts";
import type { InteractionBroker } from "../features/interactions/interaction-broker.ts";
import { HostAppImpl } from "./host-app.ts";

test("HostApp starts the runtime before listening", async () => {
  const order: string[] = [];
  const supervisor = new RuntimeSupervisor(async () => {
      order.push("runtime");
      return {
        session: {
          isIdle: true,
          sessionId: "session-1",
          sessionManager: { getCwd: () => "/workspace" },
          getActiveToolNames: () => [],
          setActiveToolsByName: () => {},
          extensionRunner: {
            hasHandlers: () => false,
            emit: async () => undefined,
          },
          dispose: () => {},
        },
        services: {},
        diagnostics: [],
      } as never;
    });
  const control = {
    listen: async () => {
      order.push("listen");
    },
    close: async () => {
      order.push("control-close");
    },
  } as unknown as ControlServer;
  const app = new HostAppImpl(
    supervisor,
    control,
    {
      recover: async () => [],
    } as unknown as IntegrationService,
    {
      hostClose: async () => {
        order.push("subagents-close");
      },
      restoreRecovered: () => {},
    } as unknown as SubagentSupervisor,
    { cancel: () => {} } as unknown as AuthService,
    { cancelAll: () => {} } as unknown as InteractionBroker,
    {
      getSessionFile: () => undefined,
      getCwd: () => "/workspace",
    } as unknown as SessionManager,
    "/workspace",
    "/agents",
  );
  await app.start();
  assert.deepEqual(order, ["runtime", "listen"]);
  await app.close();
  assert.deepEqual(order, ["runtime", "listen", "subagents-close", "control-close"]);
});

test("HostApp close is idempotent", async () => {
  let controlCloses = 0;
  const supervisor = new RuntimeSupervisor(
    async () =>
      ({
        session: {
          isIdle: true,
          sessionId: "session-1",
          sessionManager: { getCwd: () => "/workspace" },
          getActiveToolNames: () => [],
          setActiveToolsByName: () => {},
          extensionRunner: {
            hasHandlers: () => false,
            emit: async () => undefined,
          },
          dispose: () => {},
        },
        services: {},
        diagnostics: [],
      }) as never,
  );
  const app = new HostAppImpl(
    supervisor,
    {
      listen: async () => undefined,
      close: async () => {
        controlCloses += 1;
      },
    } as unknown as ControlServer,
    { recover: async () => [] } as unknown as IntegrationService,
    { hostClose: async () => undefined, restoreRecovered: () => {} } as unknown as SubagentSupervisor,
    { cancel: () => {} } as unknown as AuthService,
    { cancelAll: () => {} } as unknown as InteractionBroker,
    {
      getSessionFile: () => undefined,
      getCwd: () => "/workspace",
    } as unknown as SessionManager,
    "/workspace",
    "/agents",
  );
  await app.close();
  await app.close();
  assert.equal(controlCloses, 2);
});
