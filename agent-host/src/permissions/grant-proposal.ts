import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";

import type {
  CapabilityMatcher,
  GrantBundle,
  InvalidationKey,
  PermissionIntent,
} from "./model.ts";
import { digestValue } from "./shell/digest.ts";
import { fileDigest, workspaceInvalidationKeys, type WorkspaceIdentity } from "./workspace-identity.ts";

const MANIFESTS_BY_EXECUTABLE: Record<string, string[]> = {
  npm: ["package.json", "package-lock.json", "npm-shrinkwrap.json"],
  npx: ["package.json", "package-lock.json", "npm-shrinkwrap.json"],
  pnpm: ["package.json", "pnpm-lock.yaml", "pnpm-workspace.yaml"],
  yarn: ["package.json", "yarn.lock"],
  cargo: ["Cargo.toml", "Cargo.lock"],
};

export function exactMatchers(intent: PermissionIntent): CapabilityMatcher[] {
  return intent.atoms.map((atom): CapabilityMatcher => {
    switch (atom.kind) {
      case "exec":
        return {
          kind: "exec",
          executable: atom.executable,
          argv: atom.argv,
          cwd: atom.cwd,
          environment: atom.environment,
        };
      case "file":
        return {
          kind: "file",
          operation: atom.operation,
          path: atom.path,
          ...(atom.destination ? { destination: atom.destination } : {}),
        };
      case "network":
        return { ...atom };
      case "opaque_code":
        return {
          kind: "opaque_code",
          runtime: atom.runtime,
          digest: atom.digest,
        };
    }
  });
}

export function proposeGrantBundles(
  intent: PermissionIntent,
  identity?: WorkspaceIdentity,
): GrantBundle[] {
  const matchers = exactMatchers(intent);
  const base = {
    workspaceId: intent.workspaceId,
    matchers,
  };
  const once: GrantBundle = { ...base, scope: "once" };
  if (intent.atoms.some((atom) => atom.kind === "opaque_code")) {
    return [once];
  }
  return [
    once,
    { ...base, scope: "session", sessionId: intent.sessionId },
    {
      ...base,
      scope: "workspace",
      invalidationKeys: identity
        ? [
            ...workspaceInvalidationKeys(identity),
            ...manifestKeys(intent, identity),
          ]
        : manifestKeys(intent),
    },
  ];
}

function manifestKeys(
  intent: PermissionIntent,
  identity?: WorkspaceIdentity,
): InvalidationKey[] {
  const keys = new Map<string, InvalidationKey>();
  for (const atom of intent.atoms) {
    if (atom.kind !== "exec") continue;
    const executable = atom.executable.split("/").at(-1) ?? atom.executable;
    const paths = (MANIFESTS_BY_EXECUTABLE[executable] ?? [])
      .map((name) => join(atom.cwd, name));
    if (executable === "cargo") {
      paths.push(...cargoInvalidationPaths(atom.cwd, identity?.canonicalPath));
    }
    for (const path of paths) {
      addFileDigest(keys, path);
    }
    if (executable === "npm") {
      const script = npmScriptName(atom.argv);
      const path = join(atom.cwd, "package.json");
      const value = script ? npmScriptDigest(path, script) : undefined;
      if (script && value) {
        keys.set(`npm:${path}:${script}`, {
          kind: "npm_script_digest",
          path,
          selector: script,
          value,
        });
      }
    }
  }
  return [...keys.values()];
}

function addFileDigest(
  keys: Map<string, InvalidationKey>,
  path: string,
): void {
  if (!existsSync(path)) return;
  const value = fileDigest(path);
  if (value) keys.set(`file:${path}`, { kind: "file_digest", path, value });
}

function npmScriptName(argv: readonly string[]): string | undefined {
  if (argv[0] === "run" || argv[0] === "run-script") return argv[1];
  if (
    argv.length === 1 &&
    !["install", "ci", "publish", "pack", "exec"].includes(argv[0] ?? "")
  ) {
    return argv[0];
  }
  return undefined;
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

function cargoInvalidationPaths(
  cwd: string,
  workspaceRoot?: string,
): string[] {
  const result: string[] = [];
  const boundary = workspaceRoot ? resolve(workspaceRoot) : undefined;
  let current = resolve(cwd);
  while (true) {
    result.push(
      join(current, "Cargo.toml"),
      join(current, "Cargo.lock"),
      join(current, "build.rs"),
      join(current, ".cargo", "config"),
      join(current, ".cargo", "config.toml"),
    );
    if (current === boundary) break;
    const parent = dirname(current);
    if (parent === current || (boundary && !current.startsWith(boundary))) break;
    current = parent;
  }
  return result;
}
