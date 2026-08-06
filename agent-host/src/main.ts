import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { getAgentDir, runRpcMode } from "@earendil-works/pi-coding-agent";

import { createHostApp } from "./app/create-host-app.ts";
import { installHostLifecycle } from "./app/host-lifecycle.ts";

const isMain =
  typeof process.argv[1] === "string" &&
  resolve(process.argv[1]) === fileURLToPath(import.meta.url);

if (isMain) {
  const socketPath = process.env.NABLA_CONTROL_SOCKET;
  if (!socketPath) throw new Error("NABLA_CONTROL_SOCKET is required");

  const app = await createHostApp({
    socketPath,
    cwd: process.cwd(),
    agentDir: getAgentDir(),
    env: process.env,
  });
  installHostLifecycle(app);
  await app.start();
  await runRpcMode(app.runtime());
}
