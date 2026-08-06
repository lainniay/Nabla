import type { HostApp } from "./host-app.ts";

export function installHostLifecycle(app: HostApp): void {
  const shutdown = () => {
    void app.close().finally(() => process.exit(0));
  };
  process.once("SIGINT", shutdown);
  process.once("SIGTERM", shutdown);
}
