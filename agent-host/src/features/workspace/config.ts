import { existsSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

import { writeAtomicJsonSync } from "../../persistence/atomic-json.ts";
import { canonicalPath } from "../permissions/filesystem/path.ts";
import {
  isJsonObject as isRecord,
  stringArray,
  errorMessage,
  validAgentName,
} from "../../protocol/validation.ts";
import {
  DEFAULT_CONFIG,
  mergeAgentDirectory,
  mergeAgentProfile,
} from "../subagents/profile-loader.ts";
import type {
  AgentConfigDiagnostic,
  AgentPermissions,
  AgentProfile,
} from "../subagents/profile-model.ts";

export interface HarnessConfig {
  schemaVersion: 2;
  maxParallel: number;
  trustedWorkspaces: string[];
  allowedProjectExtensions: string[];
  profiles: Record<string, AgentProfile>;
  diagnostics: AgentConfigDiagnostic[];
}

export interface ResourceSnapshot {
  scopeId?: string;
  trusted: boolean;
  contextFiles: string[];
  skills: Array<{ name: string; path: string; description: string }>;
  prompts: Array<{ name: string; path: string; description: string }>;
  extensions: string[];
  commands: Array<{
    name: string;
    description: string;
    source: "extension" | "prompt" | "skill";
  }>;
  diagnostics: Array<{
    type: string;
    message: string;
    path?: string;
  }>;
  revision: number;
}

interface HarnessConfigOptions {
  homeDir?: string;
}

export function loadHarnessConfig(
  cwd: string,
  options: HarnessConfigOptions = {},
): HarnessConfig {
  const home = options.homeDir ?? homedir();
  const globalPath = join(home, ".nabla", "config.json");
  const diagnostics: AgentConfigDiagnostic[] = [];
  const globalValue = readJsonObject(globalPath, diagnostics);
  let globalConfig = mergeConfig(
    cloneHarnessConfig(DEFAULT_CONFIG),
    globalValue,
    globalPath,
    diagnostics,
  );
  globalConfig = mergeAgentDirectory(
    globalConfig,
    join(home, ".nabla", "agents"),
    diagnostics,
  );
  if (!workspaceIsTrusted(cwd, globalConfig)) {
    return { ...globalConfig, diagnostics };
  }
  const projectPath = join(cwd, ".nabla", "config.json");
  const projectValue = readJsonObject(projectPath, diagnostics);
  let projectConfig = mergeConfig(
    globalConfig,
    projectValue,
    projectPath,
    diagnostics,
    true,
  );
  projectConfig = mergeAgentDirectory(
    projectConfig,
    join(cwd, ".nabla", "agents"),
    diagnostics,
  );
  return restrictProjectPermissions(globalConfig, projectConfig, diagnostics);
}

export function workspaceIsTrusted(
  cwd: string,
  config: HarnessConfig,
): boolean {
  const canonical = canonicalPath(cwd);
  return config.trustedWorkspaces.some(
    (workspace) => canonicalPath(workspace) === canonical,
  );
}

export function readConfigJson(
  path: string,
  diagnostics: AgentConfigDiagnostic[],
): Record<string, unknown> {
  if (!existsSync(path)) return {};
  try {
    const value = JSON.parse(readFileSync(path, "utf8"));
    if (isRecord(value)) return value;
    diagnostics.push({
      type: "error",
      message: "Configuration root must be a JSON object",
      path,
    });
  } catch (error) {
    diagnostics.push({
      type: "error",
      message: `Unable to parse configuration: ${errorMessage(error)}`,
      path,
    });
  }
  return {};
}

export function writeConfigJson(path: string, value: Record<string, unknown>): void {
  writeAtomicJsonSync(path, value);
}

function restrictProjectPermissions(
  globalConfig: HarnessConfig,
  projectConfig: HarnessConfig,
  diagnostics: AgentConfigDiagnostic[],
): HarnessConfig {
  const profiles = Object.fromEntries(
    Object.entries(projectConfig.profiles).map(([name, projectProfile]) => {
      const parent = globalConfig.profiles[name];
      if (!parent) {
        diagnostics.push({
          type: "warning",
          message:
            `Project profile ${name} has no user-managed parent and cannot expose tools`,
          path: projectProfile.source,
          profile: name,
        });
        return [name, { ...projectProfile, tools: [], permission: {}, disabled: true }];
      }
      const permission: AgentPermissions = {};
      for (const tool of new Set([
        ...Object.keys(parent.permission),
        ...Object.keys(projectProfile.permission),
      ])) {
        permission[tool] = [
          ...(parent.permission[tool] ?? []),
          ...(projectProfile.permission[tool] ?? []).filter(
            (rule) => rule.effect !== "allow",
          ),
        ];
      }
      return [
        name,
        {
          ...projectProfile,
          tools: projectProfile.tools.filter((tool) => parent.tools.includes(tool)),
          permission,
        },
      ];
    }),
  );
  return { ...projectConfig, profiles, diagnostics };
}

function mergeConfig(
  base: HarnessConfig,
  raw: Record<string, unknown>,
  source: string,
  diagnostics: AgentConfigDiagnostic[],
  project = false,
): HarnessConfig {
  const profiles = Object.fromEntries(
    Object.entries(base.profiles).map(([name, profile]) => [
      name,
      structuredClone(profile),
    ]),
  );
  if (isRecord(raw.profiles)) {
    for (const [name, value] of Object.entries(raw.profiles)) {
      if (!validAgentName(name)) {
        diagnostics.push({
          type: "error",
          message: `Invalid subagent name: ${name}`,
          path: source,
          profile: name,
        });
        continue;
      }
      if (!isRecord(value)) {
        diagnostics.push({
          type: "error",
          message: `Subagent ${name} must be an object`,
          path: source,
          profile: name,
        });
        continue;
      }
      profiles[name] = mergeAgentProfile(
        profiles[name],
        value,
        name,
        source,
        diagnostics,
        false,
      );
    }
  }
  const requestedMax =
    typeof raw.maxParallel === "number" &&
    Number.isInteger(raw.maxParallel) &&
    raw.maxParallel > 0
      ? raw.maxParallel
      : base.maxParallel;
  return {
    schemaVersion: 2,
    maxParallel: requestedMax,
    trustedWorkspaces: project
      ? base.trustedWorkspaces
      : stringArray(raw.trustedWorkspaces).length > 0
        ? stringArray(raw.trustedWorkspaces)
        : base.trustedWorkspaces,
    allowedProjectExtensions: project
      ? base.allowedProjectExtensions
      : stringArray(raw.allowedProjectExtensions).length > 0
        ? stringArray(raw.allowedProjectExtensions)
        : base.allowedProjectExtensions,
    profiles,
    diagnostics,
  };
}

function readJsonObject(
  path: string,
  diagnostics: AgentConfigDiagnostic[],
): Record<string, unknown> {
  return readConfigJson(path, diagnostics);
}

function cloneHarnessConfig(config: HarnessConfig): HarnessConfig {
  return structuredClone(config);
}
