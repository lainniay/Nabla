import { existsSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join, resolve } from "node:path";

import { writeAtomicJsonSync } from "../../../persistence/atomic-json.ts";
import { isJsonObject, isStringRecord } from "../../../protocol/validation.ts";
import type { GrantBundle } from "../model.ts";
import type { CapabilityMatcher, InvalidationKey } from "../model.ts";
import {
  invalidationKeysValid,
  fileDigest,
  resolveWorkspaceIdentity,
  workspaceInvalidationKeys,
  type WorkspaceIdentity,
} from "../workspace-identity.ts";
import {
  canonicalJson,
  digestValue,
  sha256Hex,
} from "../shell/digest.ts";

interface WorkspaceGrantDocument {
  schemaVersion: 3;
  identity: {
    canonicalRoot: string;
    generationId: string;
    gitCommonDirectory?: string;
    gitCommonDirectoryIdentity?: string;
  };
  grants: GrantBundle[];
}

export interface WorkspaceGrantRecord extends GrantBundle {
  id: string;
}

export interface WorkspaceGrantSnapshot {
  workspace: string;
  grants: WorkspaceGrantRecord[];
}

export class WorkspaceGrantStore {
  private readonly root: string;
  private readonly legacyV2Path: string;
  private readonly legacyPath: string;

  constructor(homeDir = homedir()) {
    this.root = join(homeDir, ".nabla", "workspaces");
    this.legacyV2Path = join(homeDir, ".nabla", "permissions.json");
    this.legacyPath = join(homeDir, ".nabla", "approvals.json");
  }

  add(bundle: GrantBundle, identity: WorkspaceIdentity): void {
    if (bundle.scope !== "workspace") {
      throw new Error("Workspace store only accepts workspace grants");
    }
    if (bundle.workspaceId !== identity.id) {
      throw new Error("Workspace grant identity does not match its store");
    }
    const document = this.read(identity);
    if (!document.grants.some((grant) => sameGrant(grant, bundle))) {
      document.grants.push(bundle);
      writeAtomicJsonSync(this.path(identity.id), document);
    }
  }

  get(identity: WorkspaceIdentity): GrantBundle[] {
    return this.read(identity).grants.filter(
      (grant) =>
        grant.workspaceId === identity.id &&
        invalidationKeysValid(grant.invalidationKeys, identity),
    );
  }

  snapshot(identity: WorkspaceIdentity): WorkspaceGrantSnapshot {
    return {
      workspace: identity.canonicalPath,
      grants: this.get(identity).map((grant) => ({
        id: digestValue(grant),
        ...grant,
      })),
    };
  }

  revoke(
    identity: WorkspaceIdentity,
    grantId: string,
  ): WorkspaceGrantSnapshot {
    const document = this.read(identity);
    document.grants = document.grants.filter(
      (grant) => digestValue(grant) !== grantId,
    );
    writeAtomicJsonSync(this.path(identity.id), document);
    return this.snapshot(identity);
  }

  clear(identity: WorkspaceIdentity): WorkspaceGrantSnapshot {
    writeAtomicJsonSync(this.path(identity.id), emptyDocument(identity));
    return this.snapshot(identity);
  }

  path(workspaceId: string): string {
    return join(this.root, workspaceId, "permissions.json");
  }

  private read(identity: WorkspaceIdentity): WorkspaceGrantDocument {
    const path = this.path(identity.id);
    if (!existsSync(path)) {
      const migrated = this.migrateLegacy(identity);
      if (migrated.grants.length > 0) writeAtomicJsonSync(path, migrated);
      return migrated;
    }
    try {
      const value: unknown = JSON.parse(readFileSync(path, "utf8"));
      if (
        !isJsonObject(value) ||
        value.schemaVersion !== 3 ||
        !isJsonObject(value.identity) ||
        !Array.isArray(value.grants)
      ) {
        return emptyDocument(identity);
      }
      const storedIdentity = value.identity;
      if (
        storedIdentity.canonicalRoot !== identity.canonicalPath ||
        storedIdentity.generationId !== identity.generationId ||
        storedIdentity.gitCommonDirectory !== identity.gitCommonDirectory ||
        storedIdentity.gitCommonDirectoryIdentity !==
          identity.gitCommonDirectoryIdentity
      ) {
        return emptyDocument(identity);
      }
      return {
        schemaVersion: 3,
        identity: {
          canonicalRoot: identity.canonicalPath,
          generationId: identity.generationId,
          ...(identity.gitCommonDirectory
            ? { gitCommonDirectory: identity.gitCommonDirectory }
            : {}),
          ...(identity.gitCommonDirectoryIdentity
            ? {
                gitCommonDirectoryIdentity:
                  identity.gitCommonDirectoryIdentity,
              }
            : {}),
        },
        grants: value.grants.filter(isGrantBundle),
      };
    } catch {
      return emptyDocument(identity);
    }
  }

  private migrateLegacy(identity: WorkspaceIdentity): WorkspaceGrantDocument {
    const document = emptyDocument(identity);
    document.grants.push(...this.migrateV2(identity));
    if (!existsSync(this.legacyPath)) return document;
    try {
      const value: unknown = JSON.parse(readFileSync(this.legacyPath, "utf8"));
      if (!isJsonObject(value) || !Array.isArray(value.rules)) {
        return document;
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
        if (identity.id !== documentIdentityId(document)) return [];
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
      document.grants.push(...grants);
      return document;
    } catch {
      return document;
    }
  }

  private migrateV2(identity: WorkspaceIdentity): GrantBundle[] {
    if (!existsSync(this.legacyV2Path)) return [];
    try {
      const value: unknown = JSON.parse(readFileSync(this.legacyV2Path, "utf8"));
      if (
        !isJsonObject(value) ||
        value.schemaVersion !== 2 ||
        !Array.isArray(value.grants)
      ) {
        return [];
      }
      return value.grants.filter(isGrantBundle).filter(
        (grant) =>
          grant.workspaceId === identity.id &&
          invalidationKeysValid(grant.invalidationKeys, identity),
      );
    } catch {
      return [];
    }
  }
}

function emptyDocument(identity: WorkspaceIdentity): WorkspaceGrantDocument {
  return {
    schemaVersion: 3,
    identity: {
      canonicalRoot: identity.canonicalPath,
      generationId: identity.generationId,
      ...(identity.gitCommonDirectory
        ? { gitCommonDirectory: identity.gitCommonDirectory }
        : {}),
      ...(identity.gitCommonDirectoryIdentity
        ? {
            gitCommonDirectoryIdentity: identity.gitCommonDirectoryIdentity,
          }
        : {}),
    },
    grants: [],
  };
}

function documentIdentityId(document: WorkspaceGrantDocument): string {
  return sha256Hex(JSON.stringify({
    canonicalPath: document.identity.canonicalRoot,
    generationId: document.identity.generationId,
    gitCommonDirectory: document.identity.gitCommonDirectory,
    gitCommonDirectoryIdentity:
      document.identity.gitCommonDirectoryIdentity,
  }));
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
    return { kind: "shell_digest", digest: digestValue({ command: value }) };
  }
  if (kind === "input") {
    return {
      kind: "tool",
      tool,
      inputDigest: sha256Hex(value),
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
  if (value.kind === "shell_digest") {
    return typeof value.digest === "string";
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
      [
        "read",
        "list",
        "create",
        "write",
        "truncate",
        "append",
        "rename",
        "delete",
      ]
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
  if (
    !isJsonObject(value) ||
    ![
      "file_digest",
      "npm_script_digest",
      "workspace_generation",
      "git_common_directory",
    ].includes(String(value.kind)) ||
    typeof value.value !== "string" ||
    (value.path !== undefined && typeof value.path !== "string")
  ) {
    return false;
  }
  return (
    value.kind !== "npm_script_digest" ||
    (typeof value.path === "string" && typeof value.selector === "string")
  );
}

function sameGrant(left: GrantBundle, right: GrantBundle): boolean {
  return canonicalJson(left) === canonicalJson(right);
}
