import { spawnSync } from "node:child_process";
import { tmpdir } from "node:os";
import { join } from "node:path";

const testRoot = join(tmpdir(), "nabla-test-agent");

const result = spawnSync(
  process.execPath,
  ["--test", "src/**/*.test.ts"],
  {
    stdio: "inherit",
    env: {
      ...process.env,
      PI_CODING_AGENT_DIR: testRoot,
      PI_CODING_AGENT_SESSION_DIR: join(testRoot, "sessions"),
    },
  },
);

process.exit(result.status ?? 1);
