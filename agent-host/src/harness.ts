import {
  existsSync,
  readFileSync,
  readdirSync,
  realpathSync,
} from "node:fs";
import { homedir } from "node:os";
import { basename, extname, join, resolve } from "node:path";

import { parseFrontmatter } from "@earendil-works/pi-coding-agent";

import type { AgentIsolationPolicy } from "./worktree.ts";
import {
  READ_ONLY_TOOL_NAMES,
  THINKING_LEVELS,
  type ThinkingLevel,
} from "./policy/tool-policy.ts";
import { writeAtomicJsonSync } from "./persistence/atomic-json.ts";
import {
  isPathWithin,
  workspaceRelativePath,
} from "./policy/path-boundary.ts";
import {
  isJsonObject as isRecord,
  stringArray,
} from "./protocol/validation.ts";

export type AgentPermissionEffect = "allow" | "ask" | "deny";

export interface AgentPermissionRule {
  resource: string;
  effect: AgentPermissionEffect;
}

export type AgentPermissions = Record<string, AgentPermissionRule[]>;

export interface AgentProfile {
  description: string;
  model?: string;
  thinkingLevel?: ThinkingLevel;
  instructions: string[];
  skills: string[];
  tools: string[];
  permission: AgentPermissions;
  maxParallel: number;
  maxTurns: number;
  isolation: AgentIsolationPolicy;
  disabled: boolean;
  source: string;
}

export interface AgentConfigDiagnostic {
  type: "warning" | "error";
  message: string;
  path?: string;
  profile?: string;
}

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

const SUPPORTED_AGENT_TOOLS = new Set([
  ...READ_ONLY_TOOL_NAMES,
  "edit",
  "write",
  "bash",
]);
function readOnlyPermissions(): AgentPermissions {
  return {
    read: [{ resource: "*", effect: "allow" }],
    grep: [{ resource: "*", effect: "allow" }],
    find: [{ resource: "*", effect: "allow" }],
    ls: [{ resource: "*", effect: "allow" }],
    edit: [{ resource: "*", effect: "deny" }],
    write: [{ resource: "*", effect: "deny" }],
    bash: [{ resource: "*", effect: "deny" }],
  };
}

function writeAgentPermissions(): AgentPermissions {
  return {
    ...readOnlyPermissions(),
    edit: [{ resource: "*", effect: "ask" }],
    write: [{ resource: "*", effect: "ask" }],
    bash: [{ resource: "*", effect: "ask" }],
  };
}

function safeCustomPermissions(): AgentPermissions {
  return {
    read: [{ resource: "*", effect: "allow" }],
    grep: [{ resource: "*", effect: "allow" }],
    find: [{ resource: "*", effect: "allow" }],
    ls: [{ resource: "*", effect: "allow" }],
  };
}

const DEFAULT_PROFILES: Record<string, AgentProfile> = {
  planner: {
    description: "Inspect the workspace and prepare dependency-aware plans.",
    thinkingLevel: "high",
    instructions: [
      "Inspect the workspace and produce a concrete, dependency-aware plan.",
      "Do not modify files.",
    ],
    skills: [],
    tools: ["read", "grep", "find", "ls"],
    permission: readOnlyPermissions(),
    maxParallel: 1,
    maxTurns: 12,
    isolation: { mode: "none", integration: "source" },
    disabled: false,
    source: "builtin",
  },
  worker: {
    description: "Implement bounded tasks and verify the resulting changes.",
    thinkingLevel: "high",
    instructions: [
      "Implement the assigned task completely within the configured tools and permissions.",
      "Run relevant verification and report artifact-backed evidence.",
    ],
    skills: [],
    tools: ["read", "grep", "find", "ls", "edit", "write", "bash"],
    permission: writeAgentPermissions(),
    maxParallel: 3,
    maxTurns: 32,
    isolation: { mode: "auto", integration: "source" },
    disabled: false,
    source: "builtin",
  },
  verifier: {
    description: "Run independent verification and report exact evidence.",
    thinkingLevel: "medium",
    instructions: [
      "Run the requested verification without modifying source files.",
      "Report exact commands, exit codes, and concise output.",
    ],
    skills: [],
    tools: ["read", "grep", "find", "ls", "bash"],
    permission: readOnlyPermissions(),
    maxParallel: 1,
    maxTurns: 12,
    isolation: { mode: "auto", integration: "manual" },
    disabled: false,
    source: "builtin",
  },
  reviewer: {
    description: "Review changes independently for regressions and omissions.",
    thinkingLevel: "high",
    instructions: [
      "Review the assigned task, supplied evidence, and resulting changes independently.",
      "Do not modify files. Return only structured findings and a verdict.",
    ],
    skills: [],
    tools: ["read", "grep", "find", "ls", "bash"],
    permission: readOnlyPermissions(),
    maxParallel: 1,
    maxTurns: 12,
    isolation: { mode: "none", integration: "source" },
    disabled: false,
    source: "builtin",
  },
};

const DEFAULT_CONFIG: HarnessConfig = {
  schemaVersion: 2,
  maxParallel: 3,
  trustedWorkspaces: [],
  allowedProjectExtensions: [],
  profiles: DEFAULT_PROFILES,
  diagnostics: [],
};

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
  const canonicalWorkspace = canonicalPath(cwd);
  const trusted = globalConfig.trustedWorkspaces.some(
    (workspace) => canonicalPath(workspace) === canonicalWorkspace,
  );
  if (!trusted) return { ...globalConfig, diagnostics };
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

export function saveWorkspaceTrust(
  cwd: string,
  trusted: boolean,
  options: HarnessConfigOptions = {},
): HarnessConfig {
  const home = options.homeDir ?? homedir();
  const path = join(home, ".nabla", "config.json");
  const raw = readJsonObject(path, []);
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
  writeAtomicJsonSync(path, next);
  return loadHarnessConfig(cwd, options);
}

export function workspaceIsTrusted(cwd: string, config: HarnessConfig): boolean {
  const canonical = canonicalPath(cwd);
  return config.trustedWorkspaces.some(
    (workspace) => canonicalPath(workspace) === canonical,
  );
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

export function modelReference(profile: AgentProfile): {
  provider: string;
  id: string;
} | undefined {
  const reference = profile.model?.trim();
  if (!reference) return undefined;
  const slash = reference.indexOf("/");
  if (slash <= 0 || slash === reference.length - 1) {
    throw new Error(`Agent model must use provider/model format: ${reference}`);
  }
  return {
    provider: reference.slice(0, slash),
    id: reference.slice(slash + 1),
  };
}

export function agentPermissionEffect(
  profile: AgentProfile,
  tool: string,
  resource = "*",
): AgentPermissionEffect {
  const fallback = READ_ONLY_TOOL_NAMES.includes(
    tool as (typeof READ_ONLY_TOOL_NAMES)[number],
  )
    ? "allow"
    : "ask";
  const effects = (profile.permission[tool] ?? [])
    .filter((rule) => agentResourceMatches(rule.resource, resource))
    .map((rule) => rule.effect);
  if (effects.includes("deny")) return "deny";
  if (effects.includes("ask")) return "ask";
  if (effects.includes("allow")) return "allow";
  return fallback;
}

export function agentPermissionSummary(profile: AgentProfile): string {
  return profile.tools
    .map((tool) => `${tool}:${agentPermissionEffect(profile, tool)}`)
    .join(",");
}

export function pathAllowedByGrant(
  cwd: string,
  path: string,
  allowedPaths: readonly string[],
): boolean {
  const root = resolve(cwd);
  const target = resolve(root, path);
  let normalized: string;
  try {
    normalized = workspaceRelativePath(root, target);
  } catch {
    return false;
  }
  return allowedPaths.some((pattern) => {
    const clean = pattern
      .trim()
      .replace(/^\.\//u, "")
      .replace(/\\/gu, "/")
      .replace(/\/\*\*$/u, "")
      .replace(/\/+$/u, "");
    if (clean === "" || clean === ".") return true;
    return normalized === clean || normalized.startsWith(`${clean}/`);
  });
}

export function isCredentialPath(path: string): boolean {
  const normalized = path.replace(/\\/gu, "/").toLocaleLowerCase();
  return [
    "/.ssh/",
    "/.aws/",
    "/.config/gcloud/",
    "/credentials",
    "/auth.json",
    "/.env",
  ].some((marker) => normalized.includes(marker));
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

function mergeAgentDirectory(
  base: HarnessConfig,
  directory: string,
  diagnostics: AgentConfigDiagnostic[],
): HarnessConfig {
  if (!existsSync(directory)) return base;
  let entries;
  try {
    entries = readdirSync(directory, { withFileTypes: true })
      .filter((entry) => entry.isFile() && extname(entry.name) === ".md")
      .sort((left, right) => left.name.localeCompare(right.name));
  } catch (error) {
    diagnostics.push({
      type: "error",
      message: `Unable to read subagent directory: ${errorMessage(error)}`,
      path: directory,
    });
    return base;
  }
  const profiles = Object.fromEntries(
    Object.entries(base.profiles).map(([name, profile]) => [
      name,
      structuredClone(profile),
    ]),
  );
  for (const entry of entries) {
    const path = join(directory, entry.name);
    const name = basename(entry.name, ".md");
    if (!validAgentName(name)) {
      diagnostics.push({
        type: "error",
        message: `Invalid subagent filename: ${entry.name}`,
        path,
        profile: name,
      });
      continue;
    }
    try {
      const parsed = parseFrontmatter(readFileSync(path, "utf8"));
      if (!isRecord(parsed.frontmatter)) {
        throw new Error("frontmatter must be an object");
      }
      const raw = { ...parsed.frontmatter };
      const body = parsed.body.trim();
      const existing = profiles[name];
      if (body) raw.prompt = body;
      if (!existing) {
        if (typeof raw.description !== "string" || !raw.description.trim()) {
          throw new Error("new subagent requires a non-empty description");
        }
        if (!body) throw new Error("new subagent requires a non-empty prompt");
      }
      profiles[name] = mergeAgentProfile(
        existing,
        raw,
        name,
        path,
        diagnostics,
        true,
      );
    } catch (error) {
      diagnostics.push({
        type: "error",
        message: `Unable to load subagent ${name}: ${errorMessage(error)}`,
        path,
        profile: name,
      });
    }
  }
  return { ...base, profiles, diagnostics };
}

function mergeAgentProfile(
  existing: AgentProfile | undefined,
  raw: Record<string, unknown>,
  name: string,
  source: string,
  diagnostics: AgentConfigDiagnostic[],
  markdown: boolean,
): AgentProfile {
  const diagnosticStart = diagnostics.length;
  const supportedFields = new Set([
    "description",
    "model",
    "thinkingLevel",
    "prompt",
    "instructions",
    "skills",
    "tools",
    "permission",
    "maxParallel",
    "maxTurns",
    "isolation",
    "disabled",
  ]);
  const unknownFields = Object.keys(raw).filter(
    (field) => !supportedFields.has(field),
  );
  if (unknownFields.length > 0) {
    diagnostics.push({
      type: "error",
      message: `Subagent ${name} has unsupported fields: ${unknownFields.join(", ")}`,
      path: source,
      profile: name,
    });
  }
  const base =
    existing ??
    ({
      description: `Custom subagent ${name}`,
      instructions: [
        "Complete the assigned task and return structured evidence.",
      ],
      skills: [],
      tools: [...READ_ONLY_TOOL_NAMES],
      permission: safeCustomPermissions(),
      maxParallel: 1,
      maxTurns: 24,
      isolation: { mode: "none", integration: "source" },
      disabled: false,
      source,
    } satisfies AgentProfile);
  const next = structuredClone(base);
  next.source = source;

  if (hasOwn(raw, "description")) {
    if (typeof raw.description === "string" && raw.description.trim()) {
      next.description = raw.description.trim();
    } else {
      configFieldError(diagnostics, source, name, "description");
    }
  }
  if (hasOwn(raw, "model")) {
    if (raw.model === null || raw.model === "") {
      delete next.model;
    } else if (
      typeof raw.model === "string" &&
      validModelReference(raw.model)
    ) {
      next.model = raw.model.trim();
    } else {
      configFieldError(diagnostics, source, name, "model");
    }
  }
  if (hasOwn(raw, "thinkingLevel")) {
    if (isThinkingLevel(raw.thinkingLevel)) {
      next.thinkingLevel = raw.thinkingLevel;
    } else if (raw.thinkingLevel === null) {
      delete next.thinkingLevel;
    } else {
      configFieldError(diagnostics, source, name, "thinkingLevel");
    }
  }
  if (hasOwn(raw, "prompt")) {
    if (typeof raw.prompt === "string" && raw.prompt.trim()) {
      next.instructions = [raw.prompt.trim()];
    } else {
      configFieldError(diagnostics, source, name, "prompt");
    }
  } else if (hasOwn(raw, "instructions")) {
    const instructions = stringArray(raw.instructions).map((item) => item.trim());
    if (instructions.length > 0) next.instructions = instructions;
    else configFieldError(diagnostics, source, name, "instructions");
  }
  if (hasOwn(raw, "skills")) {
    if (Array.isArray(raw.skills)) next.skills = normalizeStrings(stringArray(raw.skills));
    else configFieldError(diagnostics, source, name, "skills");
  }
  if (hasOwn(raw, "tools")) {
    if (Array.isArray(raw.tools)) {
      const requested = normalizeStrings(stringArray(raw.tools));
      const unsupported = requested.filter(
        (tool) => !SUPPORTED_AGENT_TOOLS.has(tool),
      );
      if (unsupported.length > 0) {
        diagnostics.push({
          type: "error",
          message: `Subagent ${name} uses unsupported tools: ${unsupported.join(", ")}`,
          path: source,
          profile: name,
        });
      } else {
        next.tools = requested;
      }
    } else {
      configFieldError(diagnostics, source, name, "tools");
    }
  }
  if (hasOwn(raw, "permission")) {
    next.permission = mergePermissions(
      next.permission,
      raw.permission,
      source,
      name,
      diagnostics,
    );
  }
  if (hasOwn(raw, "maxParallel")) {
    if (positiveInteger(raw.maxParallel)) next.maxParallel = raw.maxParallel;
    else configFieldError(diagnostics, source, name, "maxParallel");
  }
  if (hasOwn(raw, "maxTurns")) {
    if (positiveInteger(raw.maxTurns)) next.maxTurns = raw.maxTurns;
    else configFieldError(diagnostics, source, name, "maxTurns");
  }
  if (hasOwn(raw, "isolation")) {
    const isolation = normalizeIsolationPolicy(raw.isolation, next.isolation);
    if (isolation) next.isolation = isolation;
    else configFieldError(diagnostics, source, name, "isolation");
  }
  if (hasOwn(raw, "disabled")) {
    if (typeof raw.disabled === "boolean") next.disabled = raw.disabled;
    else configFieldError(diagnostics, source, name, "disabled");
  }
  if (markdown && next.instructions.every((item) => !item.trim())) {
    throw new Error("subagent prompt must not be empty");
  }
  if (
    diagnostics
      .slice(diagnosticStart)
      .some(
        (diagnostic) =>
          diagnostic.type === "error" && diagnostic.profile === name,
      )
  ) {
    next.disabled = true;
  }
  return next;
}

function mergePermissions(
  base: AgentPermissions,
  value: unknown,
  source: string,
  profile: string,
  diagnostics: AgentConfigDiagnostic[],
): AgentPermissions {
  if (value === "read_only") return readOnlyPermissions();
  if (!isRecord(value)) {
    configFieldError(diagnostics, source, profile, "permission");
    return base;
  }
  const next = structuredClone(base);
  for (const [tool, candidate] of Object.entries(value)) {
    if (!SUPPORTED_AGENT_TOOLS.has(tool)) {
      diagnostics.push({
        type: "error",
        message: `Subagent ${profile} has permission for unsupported tool: ${tool}`,
        path: source,
        profile,
      });
      continue;
    }
    const rules = permissionRules(candidate);
    if (!rules) {
      diagnostics.push({
        type: "error",
        message: `Subagent ${profile} has invalid permission rules for ${tool}`,
        path: source,
        profile,
      });
      continue;
    }
    next[tool] = rules;
  }
  return next;
}

function permissionRules(value: unknown): AgentPermissionRule[] | undefined {
  if (isPermissionEffect(value)) {
    return [{ resource: "*", effect: value }];
  }
  if (!isRecord(value)) return undefined;
  const rules: AgentPermissionRule[] = [];
  for (const [resource, effect] of Object.entries(value)) {
    if (!resource.trim() || !isPermissionEffect(effect)) return undefined;
    rules.push({ resource: resource.trim(), effect });
  }
  return rules.length > 0 ? rules : undefined;
}

function normalizeIsolationPolicy(
  value: unknown,
  base: AgentIsolationPolicy,
): AgentIsolationPolicy | undefined {
  if (value === "none" || value === "auto" || value === "worktree") {
    return { ...base, mode: value };
  }
  if (!isRecord(value)) return undefined;
  const mode =
    value.mode === undefined
      ? base.mode
      : value.mode === "none" ||
          value.mode === "auto" ||
          value.mode === "worktree"
        ? value.mode
        : undefined;
  const integration =
    value.integration === undefined
      ? base.integration
      : value.integration === "source" ||
          value.integration === "auto" ||
          value.integration === "ask" ||
          value.integration === "manual"
        ? value.integration
        : undefined;
  if (!mode || !integration) return undefined;
  const unknownFields = Object.keys(value).filter(
    (field) => field !== "mode" && field !== "integration",
  );
  return unknownFields.length === 0 ? { mode, integration } : undefined;
}

function readJsonObject(
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

function cloneHarnessConfig(config: HarnessConfig): HarnessConfig {
  return structuredClone(config);
}

function agentResourceMatches(pattern: string, resource: string): boolean {
  if (pattern === "*") return true;
  const expression = pattern
    .replace(/[.+^${}()|[\]\\]/gu, "\\$&")
    .replace(/\*\*/gu, "\u0000")
    .replace(/\*/gu, ".*")
    .replace(/\u0000/gu, ".*")
    .replace(/\?/gu, ".");
  try {
    return new RegExp(`^${expression}$`, "u").test(resource);
  } catch {
    return false;
  }
}

function validAgentName(name: string): boolean {
  return /^[a-z0-9][a-z0-9_-]*$/u.test(name);
}

function validModelReference(value: string): boolean {
  const slash = value.trim().indexOf("/");
  return slash > 0 && slash < value.trim().length - 1;
}

function positiveInteger(value: unknown): value is number {
  return typeof value === "number" && Number.isInteger(value) && value > 0;
}

function isPermissionEffect(value: unknown): value is AgentPermissionEffect {
  return value === "allow" || value === "ask" || value === "deny";
}

function hasOwn(record: Record<string, unknown>, key: string): boolean {
  return Object.prototype.hasOwnProperty.call(record, key);
}

function configFieldError(
  diagnostics: AgentConfigDiagnostic[],
  path: string,
  profile: string,
  field: string,
): void {
  diagnostics.push({
    type: "error",
    message: `Subagent ${profile} has invalid ${field}`,
    path,
    profile,
  });
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function canonicalPath(path: string): string {
  try {
    return realpathSync(path);
  } catch {
    return resolve(path);
  }
}

function normalizeStrings(values: readonly string[]): string[] {
  return [...new Set(values.map((value) => value.trim()).filter(Boolean))];
}

function isThinkingLevel(value: unknown): value is AgentProfile["thinkingLevel"] {
  return THINKING_LEVELS.includes(String(value) as ThinkingLevel);
}
