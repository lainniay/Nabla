import { existsSync, readdirSync, readFileSync } from "node:fs";
import { basename, extname, join } from "node:path";

import { parseFrontmatter } from "@earendil-works/pi-coding-agent";

import type { AgentIsolationPolicy } from "./isolation/model.ts";
import {
  READ_ONLY_TOOL_NAMES,
  THINKING_LEVELS,
  type ThinkingLevel,
} from "../permissions/shell/rules.ts";
import type { HarnessConfig } from "../workspace/config.ts";
import { EMPTY_SANDBOX_CONFIG } from "../permissions/execution/sandbox-config.ts";
import {
  isJsonObject as isRecord,
  stringArray,
  errorMessage,
  validAgentName,
} from "../../protocol/validation.ts";
import {
  readOnlyPermissions,
  safeCustomPermissions,
  writeAgentPermissions,
  SUPPORTED_AGENT_TOOLS,
  type AgentConfigDiagnostic,
  type AgentPermissionRule,
  type AgentPermissions,
  type AgentProfile,
} from "./profile-model.ts";

export const DEFAULT_PROFILES: Record<string, AgentProfile> = {
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

export const DEFAULT_CONFIG: HarnessConfig = {
  schemaVersion: 2,
  maxParallel: 3,
  trustedWorkspaces: [],
  allowedProjectExtensions: [],
  profiles: DEFAULT_PROFILES,
  sandbox: EMPTY_SANDBOX_CONFIG,
  diagnostics: [],
};

export function mergeAgentDirectory(
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

export function mergeAgentProfile(
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

function validModelReference(value: string): boolean {
  const slash = value.trim().indexOf("/");
  return slash > 0 && slash < value.trim().length - 1;
}

function positiveInteger(value: unknown): value is number {
  return typeof value === "number" && Number.isInteger(value) && value > 0;
}

function isPermissionEffect(value: unknown): value is AgentPermissionRule["effect"] {
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

function normalizeStrings(values: readonly string[]): string[] {
  return [...new Set(values.map((value) => value.trim()).filter(Boolean))];
}

function isThinkingLevel(value: unknown): value is ThinkingLevel {
  return THINKING_LEVELS.includes(String(value) as ThinkingLevel);
}
