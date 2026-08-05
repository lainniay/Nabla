import { createHash, randomUUID } from "node:crypto";
import {
  existsSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  realpathSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { dirname, isAbsolute, join, resolve } from "node:path";

import type { InvalidationKey } from "./model.ts";
import { digestValue } from "./shell/digest.ts";

export interface WorkspaceIdentity {
  id: string;
  canonicalPath: string;
  generationId: string;
  gitCommonDirectory?: string;
  gitCommonDirectoryIdentity?: string;
}

export function resolveWorkspaceIdentity(
  workspace: string,
  generationFile?: string,
): WorkspaceIdentity {
  const canonicalPath = canonical(workspace);
  const generationId = generationFile
    ? persistentGeneration(generationFile)
    : filesystemGeneration(canonicalPath);
  const gitCommonDirectory = findGitCommonDirectory(canonicalPath);
  const gitCommonDirectoryIdentity = gitCommonDirectory
    ? filesystemGeneration(gitCommonDirectory)
    : undefined;
  const id = digest({
    canonicalPath,
    generationId,
    gitCommonDirectory,
    gitCommonDirectoryIdentity,
  });
  return {
    id,
    canonicalPath,
    generationId,
    ...(gitCommonDirectory ? { gitCommonDirectory } : {}),
    ...(gitCommonDirectoryIdentity ? { gitCommonDirectoryIdentity } : {}),
  };
}

export function workspaceInvalidationKeys(
  identity: WorkspaceIdentity,
): InvalidationKey[] {
  return [
    { kind: "workspace_generation", value: identity.generationId },
    ...(identity.gitCommonDirectoryIdentity
      ? [{
          kind: "git_common_directory" as const,
          path: identity.gitCommonDirectory,
          value: identity.gitCommonDirectoryIdentity,
        }]
      : []),
  ];
}

export function fileDigest(path: string): string | undefined {
  try {
    return createHash("sha256").update(readFileSync(path)).digest("hex");
  } catch {
    return undefined;
  }
}

export function invalidationKeysValid(
  keys: readonly InvalidationKey[] | undefined,
  identity: WorkspaceIdentity,
): boolean {
  return (keys ?? []).every((key) => {
    if (key.kind === "workspace_generation") {
      return key.value === identity.generationId;
    }
    if (key.kind === "git_common_directory") {
      return key.value === identity.gitCommonDirectoryIdentity;
    }
    if (key.kind === "npm_script_digest") {
      return key.path !== undefined &&
        key.selector !== undefined &&
        npmScriptDigest(key.path, key.selector) === key.value;
    }
    return key.path !== undefined && fileDigest(key.path) === key.value;
  });
}

function npmScriptDigest(path: string, script: string): string | undefined {
  try {
    const parsed: unknown = JSON.parse(readFileSync(path, "utf8"));
    if (
      typeof parsed !== "object" ||
      parsed === null ||
      !("scripts" in parsed) ||
      typeof parsed.scripts !== "object" ||
      parsed.scripts === null
    ) {
      return undefined;
    }
    const value = (parsed.scripts as Record<string, unknown>)[script];
    return typeof value === "string" ? digestValue(value) : undefined;
  } catch {
    return undefined;
  }
}

function findGitCommonDirectory(workspace: string): string | undefined {
  let current = workspace;
  while (true) {
    const dotGit = join(current, ".git");
    if (existsSync(dotGit)) {
      if (lstatSync(dotGit).isDirectory()) {
        const commonDirFile = join(dotGit, "commondir");
        if (existsSync(commonDirFile)) {
          return canonical(resolve(dotGit, readFileSync(commonDirFile, "utf8").trim()));
        }
        return canonical(dotGit);
      }
      const match = /^gitdir:\s*(.+)$/imu.exec(readFileSync(dotGit, "utf8"));
      if (!match) return undefined;
      const gitDir = canonical(resolve(current, match[1]!));
      const commonDirFile = join(gitDir, "commondir");
      return existsSync(commonDirFile)
        ? canonical(resolve(gitDir, readFileSync(commonDirFile, "utf8").trim()))
        : gitDir;
    }
    const parent = dirname(current);
    if (parent === current) return undefined;
    current = parent;
  }
}

function filesystemGeneration(path: string): string {
  const stat = statSync(path);
  return digest({
    device: stat.dev,
    inode: stat.ino,
    birthtime: stat.birthtimeMs,
  });
}

function persistentGeneration(path: string): string {
  try {
    const value = readFileSync(path, "utf8").trim();
    if (value) return value;
  } catch {
    // Create the user-owned generation marker below.
  }
  const value = randomUUID();
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, `${value}\n`, { mode: 0o600 });
  return value;
}

function canonical(path: string): string {
  try {
    return realpathSync(path);
  } catch {
    return isAbsolute(path) ? resolve(path) : resolve(process.cwd(), path);
  }
}

function digest(value: unknown): string {
  return createHash("sha256").update(JSON.stringify(value)).digest("hex");
}
