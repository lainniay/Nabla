import { homedir } from "node:os";
import { join, resolve } from "node:path";

import { isPathWithin } from "../permissions/filesystem/path.ts";
import { stringArray } from "../../protocol/validation.ts";
import {
  loadHarnessConfig,
  readConfigJson,
  writeConfigJson,
  type HarnessConfig,
} from "./config.ts";
import { canonicalPath } from "../permissions/filesystem/path.ts";

interface HarnessConfigOptions {
  homeDir?: string;
}

export function saveWorkspaceTrust(
  cwd: string,
  trusted: boolean,
  options: HarnessConfigOptions = {},
): HarnessConfig {
  const home = options.homeDir ?? homedir();
  const path = join(home, ".nabla", "config.json");
  const raw = readConfigJson(path, []);
  const canonical = canonicalPath(cwd);
  const workspaces = new Set(stringArray(raw.trustedWorkspaces).map(canonicalPath));
  if (trusted) workspaces.add(canonical);
  else workspaces.delete(canonical);
  const next: Record<string, unknown> = {
    ...raw,
    schemaVersion:
      typeof raw.schemaVersion === "number" ? raw.schemaVersion : 2,
    trustedWorkspaces: [...workspaces].sort(),
  };
  writeConfigJson(path, next);
  return loadHarnessConfig(cwd, options);
}

export function filterContextFilesByTrust<T extends { path: string }>(
  files: readonly T[],
  agentDir: string,
  trusted: boolean,
): T[] {
  if (trusted) return [...files];
  const root = resolve(agentDir);
  return files.filter((file) => isPathWithin(root, file.path));
}
