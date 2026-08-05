import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join, resolve } from "node:path";

import { writeAtomicJsonSync } from "../../persistence/atomic-json.ts";
import { isJsonObject } from "../../protocol/validation.ts";
import type { GrantBundle } from "../model.ts";
import type { CapabilityMatcher, InvalidationKey } from "../model.ts";
import {
  invalidationKeysValid,
  fileDigest,
  resolveWorkspaceIdentity,
  workspaceInvalidationKeys,
  type WorkspaceIdentity,
} from "../workspace-identity.ts";

interface WorkspaceGrantDocument {
  schemaVersion: 2;
  grants: GrantBundle[];
}

export class WorkspaceGrantStore {
  private readonly path: string;
  private readonly legacyPath: string;

  constructor(homeDir = homedir()) {
    this.path = join(homeDir, ".nabla", "permissions.json");
    this.legacyPath = join(homeDir, ".nabla", "approvals.json");
  }

  add(bundle: GrantBundle): void {
    if (bundle.scope !== "workspace") {
      throw new Error("Workspace store only accepts workspace grants");
    }
    const document = this.read();
    if (!document.grants.some((grant) => sameGrant(grant, bundle))) {
      document.grants.push(bundle);
      writeAtomicJsonSync(this.path, document);
    }
  }

  get(identity: WorkspaceIdentity): GrantBundle[] {
    return this.read().grants.filter(
      (grant) =>
        grant.workspaceId === identity.id &&
        invalidationKeysValid(grant.invalidationKeys, identity),
    );
  }

  private read(): WorkspaceGrantDocument {
    if (!existsSync(this.path)) {
      const migrated = this.migrateLegacy();
      if (migrated.grants.length > 0) writeAtomicJsonSync(this.path, migrated);
      return migrated;
    }
    try {
      const value: unknown = JSON.parse(readFileSync(this.path, "utf8"));
      if (!isJsonObject(value) || value.schemaVersion !== 2 ||
          !Array.isArray(value.grants)) {
        return { schemaVersion: 2, grants: [] };
      }
      return {
        schemaVersion: 2,
        grants: value.grants.filter(isGrantBundle),
      };
    } catch {
      return { schemaVersion: 2, grants: [] };
    }
  }

  private migrateLegacy(): WorkspaceGrantDocument {
    if (!existsSync(this.legacyPath)) return { schemaVersion: 2, grants: [] };
    try {
      const value: unknown = JSON.parse(readFileSync(this.legacyPath, "utf8"));
      if (!isJsonObject(value) || !Array.isArray(value.rules)) {
        return { schemaVersion: 2, grants: [] };
      }
      const grants = value.rules.flatMap((rule): GrantBundle[] => {
        if (
          !isJsonObject(rule) ||
          typeof rule.workspace !== "string" ||
          typeof rule.toolName !== "string" ||
          typeof rule.kind !== "string" ||
          typeof rule.value !== "string"
        ) {
          return [];
        }
        let identity: WorkspaceIdentity;
        try {
          identity = resolveWorkspaceIdentity(rule.workspace);
        } catch {
          return [];
        }
        const matcher = legacyMatcher(rule, identity.canonicalPath);
        return matcher
          ? [{
              scope: "workspace",
              workspaceId: identity.id,
              matchers: [matcher],
              invalidationKeys: [
                ...workspaceInvalidationKeys(identity),
                ...legacyCommandInvalidationKeys(rule, identity.canonicalPath),
              ],
            }]
          : [];
      });
      return { schemaVersion: 2, grants };
    } catch {
      return { schemaVersion: 2, grants: [] };
    }
  }
}

function legacyCommandInvalidationKeys(
  rule: Record<string, unknown>,
  workspace: string,
): InvalidationKey[] {
  if (rule.kind !== "command" || typeof rule.value !== "string") return [];
  const executable = rule.value.trim().split(/\s+/u)[0]?.split("/").at(-1);
  const manifests =
    executable === "cargo"
      ? ["Cargo.toml", "Cargo.lock"]
      : ["npm", "npx"].includes(executable ?? "")
        ? ["package.json", "package-lock.json", "npm-shrinkwrap.json"]
        : [];
  return manifests.flatMap((name): InvalidationKey[] => {
    const path = join(workspace, name);
    const value = fileDigest(path);
    return value ? [{ kind: "file_digest", path, value }] : [];
  });
}

function legacyMatcher(
  rule: Record<string, unknown>,
  workspace: string,
): CapabilityMatcher | undefined {
  const kind = rule.kind;
  const tool = String(rule.toolName);
  const value = String(rule.value);
  if (kind === "command_prefix") return undefined;
  if (kind === "command") {
    return { kind: "opaque_shell_exact", command: value };
  }
  if (kind === "input") {
    return {
      kind: "tool",
      tool,
      inputDigest: createHash("sha256").update(value).digest("hex"),
    };
  }
  if (kind !== "path") return undefined;
  const operation =
    tool === "read" || tool === "grep" || tool === "find"
      ? "read"
      : tool === "ls"
        ? "list"
        : tool === "edit" || tool === "write"
          ? "write"
          : undefined;
  if (!operation) return undefined;
  return {
    kind: "file",
    operation,
    path: resolve(workspace, value),
    recursive: rule.recursive === true,
  };
}

function isGrantBundle(value: unknown): value is GrantBundle {
  return (
    isJsonObject(value) &&
    value.scope === "workspace" &&
    typeof value.workspaceId === "string" &&
    Array.isArray(value.matchers) &&
    value.matchers.every(isCapabilityMatcher) &&
    (value.invalidationKeys === undefined ||
      (Array.isArray(value.invalidationKeys) &&
        value.invalidationKeys.every(isInvalidationKey)))
  );
}

function isCapabilityMatcher(value: unknown): value is CapabilityMatcher {
  if (!isJsonObject(value) || typeof value.kind !== "string") return false;
  if (
    value.kind === "shell_intent" ||
    value.kind === "opaque_shell_exact"
  ) {
    return typeof value.command === "string";
  }
  if (value.kind === "tool") {
    return typeof value.tool === "string" &&
      (value.inputDigest === undefined || typeof value.inputDigest === "string");
  }
  if (value.kind === "exec") {
    return (
      typeof value.executable === "string" &&
      (value.argv === undefined ||
        (Array.isArray(value.argv) &&
          value.argv.every((item) => typeof item === "string"))) &&
      (value.cwd === undefined || typeof value.cwd === "string") &&
      (value.environment === undefined || isStringRecord(value.environment))
    );
  }
  if (value.kind === "file") {
    return (
      ["read", "list", "create", "write", "append", "rename", "delete"]
        .includes(String(value.operation)) &&
      typeof value.path === "string" &&
      (value.recursive === undefined || typeof value.recursive === "boolean") &&
      (value.destination === undefined || typeof value.destination === "string")
    );
  }
  if (value.kind === "network") {
    return (
      (value.operation === "connect" || value.operation === "listen") &&
      typeof value.host === "string" &&
      (value.port === undefined || typeof value.port === "number") &&
      (value.protocol === undefined || typeof value.protocol === "string")
    );
  }
  return (
    value.kind === "opaque_code" &&
    typeof value.runtime === "string" &&
    typeof value.digest === "string"
  );
}

function isInvalidationKey(value: unknown): value is InvalidationKey {
  return (
    isJsonObject(value) &&
    ["file_digest", "workspace_generation", "git_common_directory"]
      .includes(String(value.kind)) &&
    typeof value.value === "string" &&
    (value.path === undefined || typeof value.path === "string")
  );
}

function isStringRecord(value: unknown): value is Record<string, string> {
  return (
    isJsonObject(value) &&
    Object.values(value).every((item) => typeof item === "string")
  );
}

function sameGrant(left: GrantBundle, right: GrantBundle): boolean {
  return JSON.stringify(left) === JSON.stringify(right);
}
