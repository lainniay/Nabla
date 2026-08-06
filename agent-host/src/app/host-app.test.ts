import assert from "node:assert/strict";
import test from "node:test";

import type { SessionManager } from "@earendil-works/pi-coding-agent";

import type { HostBridge } from "../legacy-host-bridge.ts";
import { RuntimeSupervisor } from "../runtime/runtime-supervisor.ts";
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
  const bridge = {
    listen: async () => {
      order.push("listen");
    },
    close: async () => {
      order.push("bridge-close");
    },
  } as unknown as HostBridge;
  const app = new HostAppImpl(
    supervisor,
    bridge,
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
  assert.deepEqual(order, ["runtime", "listen", "bridge-close"]);
});

test("HostApp close is idempotent", async () => {
  let bridgeCloses = 0;
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
  const bridge = {
    listen: async () => undefined,
    close: async () => {
      bridgeCloses += 1;
    },
  } as unknown as HostBridge;
  const app = new HostAppImpl(
    supervisor,
    bridge,
    {
      getSessionFile: () => undefined,
      getCwd: () => "/workspace",
    } as unknown as SessionManager,
    "/workspace",
    "/agents",
  );
  await app.close();
  await app.close();
  assert.equal(bridgeCloses, 2);
});
