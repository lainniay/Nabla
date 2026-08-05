import { existsSync } from "node:fs";
import { join } from "node:path";

import type {
  CapabilityMatcher,
  GrantBundle,
  InvalidationKey,
  PermissionIntent,
} from "./model.ts";
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
  return [
    { ...base, scope: "once" },
    { ...base, scope: "session", sessionId: intent.sessionId },
    {
      ...base,
      scope: "workspace",
      invalidationKeys: identity
        ? [...workspaceInvalidationKeys(identity), ...manifestKeys(intent)]
        : manifestKeys(intent),
    },
  ];
}

function manifestKeys(intent: PermissionIntent): InvalidationKey[] {
  const keys = new Map<string, InvalidationKey>();
  for (const atom of intent.atoms) {
    if (atom.kind !== "exec") continue;
    const executable = atom.executable.split("/").at(-1) ?? atom.executable;
    for (const name of MANIFESTS_BY_EXECUTABLE[executable] ?? []) {
      const path = join(atom.cwd, name);
      if (!existsSync(path)) continue;
      const value = fileDigest(path);
      if (value) keys.set(path, { kind: "file_digest", path, value });
    }
  }
  return [...keys.values()];
}
