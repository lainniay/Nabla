import { randomUUID } from "node:crypto";
import { existsSync, readFileSync, realpathSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { join, resolve } from "node:path";

import { writeAtomicJsonSync } from "./persistence/atomic-json.ts";
import { workspaceRelativePath } from "./policy/path-boundary.ts";
import {
  hasShellControlSyntax,
  isHighRiskCommand,
  isSafeReadOnlyWorkspaceCommand,
  SAFE_READ_ONLY_COMMAND_PREFIXES,
} from "./policy/tool-policy.ts";
import { isJsonObject } from "./protocol/validation.ts";

export type ApprovalRuleKind = "path" | "command" | "command_prefix" | "input";

export interface PersistentApprovalRule {
  id: string;
  workspace: string;
  toolName: string;
  kind: ApprovalRuleKind;
  value: string;
  recursive: boolean;
  summary: string;
  createdAt: string;
}

export interface ApprovalRulesSnapshot {
  workspace: string;
  rules: PersistentApprovalRule[];
}

interface ApprovalDocument {
  schemaVersion: 1;
  rules: PersistentApprovalRule[];
}

interface ApprovalStoreOptions {
  homeDir?: string;
  now?: () => string;
  createId?: () => string;
}

export class ApprovalStore {
  private readonly path: string;
  private readonly now: () => string;
  private readonly createId: () => string;
  private readonly sessionRules = new Map<string, PersistentApprovalRule[]>();

  constructor(options: ApprovalStoreOptions = {}) {
    this.path = join(options.homeDir ?? homedir(), ".nabla", "approvals.json");
    this.now = options.now ?? (() => new Date().toISOString());
    this.createId = options.createId ?? randomUUID;
  }

  allows(
    cwd: string,
    toolName: string,
    input: unknown,
  ): boolean {
    const candidate = ruleCandidate(cwd, toolName, input);
    if (!candidate) return false;
    return this.read().rules.some(
      (rule) =>
        rule.workspace === candidate.workspace &&
        rule.toolName === candidate.toolName &&
        rule.kind === candidate.kind &&
        ruleMatches(rule, candidate),
    );
  }

  allowsSession(
    sessionId: string,
    cwd: string,
    toolName: string,
    input: unknown,
  ): boolean {
    const candidate = ruleCandidate(cwd, toolName, input);
    if (!candidate) return false;
    return (this.sessionRules.get(sessionId) ?? []).some(
      (rule) =>
        rule.workspace === candidate.workspace &&
        rule.toolName === candidate.toolName &&
        rule.kind === candidate.kind &&
        ruleMatches(rule, candidate),
    );
  }

  allowSession(
    sessionId: string,
    cwd: string,
    toolName: string,
    input: unknown,
  ): void {
    const candidate = ruleCandidate(cwd, toolName, input);
    if (!candidate) {
      throw new Error("This request cannot be safely approved for the session");
    }
    const rules = this.sessionRules.get(sessionId) ?? [];
    if (
      !rules.some(
        (rule) =>
          rule.workspace === candidate.workspace &&
          rule.toolName === candidate.toolName &&
          rule.kind === candidate.kind &&
          rule.value === candidate.value &&
          rule.recursive === candidate.recursive,
      )
    ) {
      rules.push({
        id: this.createId(),
        ...candidate,
        createdAt: this.now(),
      });
      this.sessionRules.set(sessionId, rules);
    }
  }

  allow(cwd: string, toolName: string, input: unknown): PersistentApprovalRule {
    const candidate = ruleCandidate(cwd, toolName, input);
    if (!candidate) {
      throw new Error("This request cannot be safely approved forever");
    }
    const document = this.read();
    const existing = document.rules.find(
      (rule) =>
        rule.workspace === candidate.workspace &&
        rule.toolName === candidate.toolName &&
        rule.kind === candidate.kind &&
        rule.value === candidate.value &&
        rule.recursive === candidate.recursive,
    );
    if (existing) return existing;
    const rule: PersistentApprovalRule = {
      id: this.createId(),
      ...candidate,
      createdAt: this.now(),
    };
    document.rules.push(rule);
    this.write(document);
    return rule;
  }

  snapshot(cwd: string): ApprovalRulesSnapshot {
    const workspace = canonicalPath(cwd);
    return {
      workspace,
      rules: this.read().rules.filter((rule) => rule.workspace === workspace),
    };
  }

  revoke(cwd: string, ruleId: string): ApprovalRulesSnapshot {
    const workspace = canonicalPath(cwd);
    const document = this.read();
    const next = document.rules.filter(
      (rule) => !(rule.workspace === workspace && rule.id === ruleId),
    );
    if (next.length === document.rules.length) {
      throw new Error("Persistent approval rule was not found in this project");
    }
    this.write({ schemaVersion: 1, rules: next });
    return this.snapshot(cwd);
  }

  clear(cwd: string): ApprovalRulesSnapshot {
    const workspace = canonicalPath(cwd);
    const document = this.read();
    this.write({
      schemaVersion: 1,
      rules: document.rules.filter((rule) => rule.workspace !== workspace),
    });
    return { workspace, rules: [] };
  }

  private read(): ApprovalDocument {
    if (!existsSync(this.path)) return { schemaVersion: 1, rules: [] };
    try {
      const value: unknown = JSON.parse(readFileSync(this.path, "utf8"));
      if (!isJsonObject(value) || value.schemaVersion !== 1 || !Array.isArray(value.rules)) {
        return { schemaVersion: 1, rules: [] };
      }
      return {
        schemaVersion: 1,
        rules: value.rules.filter(isPersistentApprovalRule),
      };
    } catch {
      return { schemaVersion: 1, rules: [] };
    }
  }

  private write(document: ApprovalDocument): void {
    writeAtomicJsonSync(this.path, {
      schemaVersion: 1,
      rules: [...document.rules].sort((left, right) =>
        `${left.workspace}\0${left.toolName}\0${left.summary}`.localeCompare(
          `${right.workspace}\0${right.toolName}\0${right.summary}`,
        ),
      ),
    });
  }
}

type RuleCandidate = Omit<PersistentApprovalRule, "id" | "createdAt">;

function ruleCandidate(
  cwd: string,
  toolName: string,
  input: unknown,
): RuleCandidate | undefined {
  if (!isJsonObject(input)) return undefined;
  const workspace = canonicalPath(cwd);
  const path = typeof input.path === "string" ? input.path : undefined;
  if (path) {
    const absolute = resolve(workspace, path);
    let relative: string;
    try {
      relative = workspaceRelativePath(workspace, absolute);
    } catch {
      return undefined;
    }
    const recursive = statSafe(absolute)?.isDirectory() ?? false;
    return {
      workspace,
      toolName,
      kind: "path",
      value: relative,
      recursive,
      summary: recursive ? `${relative}/**` : relative,
    };
  }
  const command = typeof input.command === "string" ? input.command : undefined;
  if (command) {
    const normalized = normalizeCommand(command);
    if (
      !normalized ||
      isHighRiskCommand(normalized) ||
      (hasShellControlSyntax(command) &&
        !isSafeReadOnlyWorkspaceCommand(command, workspace))
    ) {
      return undefined;
    }
    const safePrefix = SAFE_READ_ONLY_COMMAND_PREFIXES.find(
      (prefix) => normalized === prefix || normalized.startsWith(`${prefix} `),
    );
    return {
      workspace,
      toolName,
      kind: safePrefix ? "command_prefix" : "command",
      value: safePrefix ?? normalized,
      recursive: false,
      summary: safePrefix ? `${safePrefix} …` : normalized,
    };
  }
  const value = canonicalJson(input);
  return {
    workspace,
    toolName,
    kind: "input",
    value,
    recursive: false,
    summary: value.length > 120 ? `${value.slice(0, 117)}...` : value,
  };
}

function ruleMatches(rule: PersistentApprovalRule, candidate: RuleCandidate): boolean {
  if (rule.kind === "path" && rule.recursive) {
    return (
      candidate.value === rule.value ||
      candidate.value.startsWith(`${rule.value}/`)
    );
  }
  if (rule.kind === "command_prefix") {
    return (
      candidate.kind === "command_prefix" &&
      (candidate.value === rule.value ||
        candidate.value.startsWith(`${rule.value} `))
    );
  }
  return rule.value === candidate.value;
}

function isPersistentApprovalRule(value: unknown): value is PersistentApprovalRule {
  return (
    isJsonObject(value) &&
    typeof value.id === "string" &&
    typeof value.workspace === "string" &&
    typeof value.toolName === "string" &&
    (value.kind === "path" ||
      value.kind === "command" ||
      value.kind === "command_prefix" ||
      value.kind === "input") &&
    typeof value.value === "string" &&
    typeof value.recursive === "boolean" &&
    typeof value.summary === "string" &&
    typeof value.createdAt === "string"
  );
}

function canonicalPath(path: string): string {
  try {
    return realpathSync(path);
  } catch {
    return resolve(path);
  }
}

function statSafe(path: string) {
  try {
    return statSync(path);
  } catch {
    return undefined;
  }
}

function normalizeCommand(command: string): string {
  return command.trim().replace(/\s+/gu, " ");
}

function canonicalJson(value: unknown): string {
  if (Array.isArray(value)) {
    return `[${value.map(canonicalJson).join(",")}]`;
  }
  if (isJsonObject(value)) {
    return `{${Object.keys(value)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${canonicalJson(value[key])}`)
      .join(",")}}`;
  }
  return JSON.stringify(value);
}
