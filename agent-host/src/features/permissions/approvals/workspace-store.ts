import { existsSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join, resolve } from "node:path";

import { Value } from "typebox/value";

import { writeAtomicJsonSync } from "../../../persistence/atomic-json.ts";
import { isJsonObject } from "../../../protocol/validation.ts";
import {
  GrantProposalSchema,
  InvalidationKeySchema,
} from "../../../protocol/schemas/permissions.ts";
import type { GrantBundle } from "../model.ts";
import type { CapabilityMatcher, InvalidationKey } from "../model.ts";
import {
  invalidationKeysValid,
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

function legacyMatcher(
  rule: Record<string, unknown>,
  workspace: string,
): CapabilityMatcher | undefined {
  const kind = rule.kind;
  const tool = String(rule.toolName);
  const value = String(rule.value);
  if (kind === "command_prefix" || kind === "command") return undefined;
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
    Value.Check(GrantProposalSchema, value) &&
    (value as GrantBundle).scope === "workspace"
  );
}

function isInvalidationKey(value: unknown): value is InvalidationKey {
  if (!Value.Check(InvalidationKeySchema, value)) return false;
  const key = value as InvalidationKey;
  return (
    key.kind !== "npm_script_digest" ||
    (typeof key.path === "string" && typeof key.selector === "string")
  );
}

function sameGrant(left: GrantBundle, right: GrantBundle): boolean {
  return canonicalJson(left) === canonicalJson(right);
}
