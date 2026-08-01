import { randomUUID } from "node:crypto";
import { isJsonObject as isRecord } from "./protocol/validation.ts";

export const PLAN_ENTRY_TYPE = "nabla.plan.v2";
export const LEGACY_PLAN_ENTRY_TYPE = "nabla.plan.v1";
export const PLAN_EXECUTION_MESSAGE_TYPE = "nabla.plan.execution.v1";
export const PLAN_MODE_ENTRY_TYPE = "nabla.plan-mode.v1";

export type PlanStatus = "submitted" | "executing" | "completed";

export interface PlanContent {
  title: string;
  summary: string;
  bodyMarkdown: string;
  assumptions: string[];
  testPlan: string[];
}

export interface PlanArtifactV2 extends PlanContent {
  schemaVersion: 2;
  id: string;
  revision: number;
  status: PlanStatus;
  sourceSessionId: string;
  createdAt: string;
  updatedAt: string;
  lastExecutionError?: string;
}

export interface PlanSessionEntry {
  type: string;
  customType?: string;
  data?: unknown;
}

export interface PlanModeEntry {
  active: boolean;
}

export interface RestoreResult {
  artifact?: PlanArtifactV2;
  recovered: boolean;
}

interface PlanStoreOptions {
  createId?: () => string;
  now?: () => string;
}

export class PlanStore {
  private artifact?: PlanArtifactV2;
  private readonly createId: () => string;
  private readonly now: () => string;

  constructor(options: PlanStoreOptions = {}) {
    this.createId = options.createId ?? randomUUID;
    this.now = options.now ?? (() => new Date().toISOString());
  }

  private nextTimestamp(previous?: string): string {
    const candidate = this.now();
    if (!previous) return candidate;
    const candidateMs = Date.parse(candidate);
    const previousMs = Date.parse(previous);
    if (
      Number.isFinite(candidateMs) &&
      Number.isFinite(previousMs) &&
      candidateMs <= previousMs
    ) {
      return new Date(previousMs + 1).toISOString();
    }
    return candidate;
  }

  latest(): PlanArtifactV2 | undefined {
    return this.artifact ? structuredClone(this.artifact) : undefined;
  }

  clear(): void {
    this.artifact = undefined;
  }

  restore(entries: readonly PlanSessionEntry[]): RestoreResult {
    const candidates = entries
      .filter(
        (entry) =>
          entry.type === "custom" &&
          (entry.customType === PLAN_ENTRY_TYPE ||
            entry.customType === LEGACY_PLAN_ENTRY_TYPE),
      )
      .map((entry) => normalizeStoredPlan(entry.data))
      .filter((artifact): artifact is PlanArtifactV2 => artifact !== undefined);
    const restored = candidates.at(-1);
    if (!restored) {
      this.artifact = undefined;
      return { recovered: false };
    }

    const recovered = restored.status === "executing";
    this.artifact = {
      ...structuredClone(restored),
      status: recovered ? "submitted" : restored.status,
      updatedAt: recovered
        ? this.nextTimestamp(restored.updatedAt)
        : restored.updatedAt,
      ...(recovered
        ? { lastExecutionError: "Previous Plan execution was interrupted." }
        : {}),
    };
    return { artifact: this.latest(), recovered };
  }

  adopt(artifact: PlanArtifactV2): void {
    if (!isPlanArtifact(artifact)) throw new Error("Invalid PlanArtifactV2");
    this.artifact = structuredClone(artifact);
  }

  submit(content: PlanContent, sourceSessionId: string): PlanArtifactV2 {
    if (this.artifact?.status === "executing") {
      throw new Error("Cannot revise a Plan while it is executing");
    }
    const normalized = normalizePlanContent(content);
    const previous = this.artifact;
    const timestamp = this.nextTimestamp(previous?.updatedAt);
    this.artifact = {
      schemaVersion: 2,
      id: previous?.id ?? this.createId(),
      revision: (previous?.revision ?? 0) + 1,
      status: "submitted",
      ...normalized,
      sourceSessionId,
      createdAt: previous?.createdAt ?? timestamp,
      updatedAt: timestamp,
    };
    return this.latest() as PlanArtifactV2;
  }

  markExecuting(): PlanArtifactV2 {
    if (!this.artifact) throw new Error("No Plan is submitted");
    if (this.artifact.status !== "submitted") {
      throw new Error(`Plan cannot start while it is ${this.artifact.status}`);
    }
    this.artifact = {
      ...this.artifact,
      status: "executing",
      updatedAt: this.nextTimestamp(this.artifact.updatedAt),
      lastExecutionError: undefined,
    };
    return this.latest() as PlanArtifactV2;
  }

  markSubmitted(error?: string): PlanArtifactV2 {
    if (!this.artifact) throw new Error("No Plan is available");
    if (this.artifact.status !== "executing") {
      throw new Error(`Plan cannot return to submitted while it is ${this.artifact.status}`);
    }
    this.artifact = {
      ...this.artifact,
      status: "submitted",
      updatedAt: this.nextTimestamp(this.artifact.updatedAt),
      lastExecutionError: error,
    };
    return this.latest() as PlanArtifactV2;
  }

  markCompleted(): PlanArtifactV2 {
    if (!this.artifact) throw new Error("No Plan is available");
    if (this.artifact.status !== "executing") {
      throw new Error(`Plan cannot complete while it is ${this.artifact.status}`);
    }
    this.artifact = {
      ...this.artifact,
      status: "completed",
      updatedAt: this.nextTimestamp(this.artifact.updatedAt),
      lastExecutionError: undefined,
    };
    return this.latest() as PlanArtifactV2;
  }
}

export function restorePlanMode(entries: readonly PlanSessionEntry[]): boolean {
  return (
    entries
      .filter(
        (entry) =>
          entry.type === "custom" && entry.customType === PLAN_MODE_ENTRY_TYPE,
      )
      .map((entry) => entry.data)
      .filter(isPlanModeEntry)
      .at(-1)?.active ?? false
  );
}

function isPlanModeEntry(value: unknown): value is PlanModeEntry {
  return isRecord(value) && typeof value.active === "boolean";
}

export function planExecutionPrompt(artifact: PlanArtifactV2): string {
  return [
    `Execute Nabla PlanArtifact ${artifact.id} revision ${artifact.revision}.`,
    "Treat the artifact below as the authoritative implementation request.",
    "Implement it completely, run proportionate verification, and report the outcome.",
    "",
    `# ${artifact.title}`,
    "",
    artifact.summary,
    "",
    artifact.bodyMarkdown,
    "",
    "## Assumptions",
    ...artifact.assumptions.map((item) => `- ${item}`),
    "",
    "## Test plan",
    ...artifact.testPlan.map((item) => `- ${item}`),
  ].join("\n");
}

export function isPlanArtifact(value: unknown): value is PlanArtifactV2 {
  if (!isRecord(value)) return false;
  return (
    value.schemaVersion === 2 &&
    typeof value.id === "string" &&
    value.id.length > 0 &&
    Number.isInteger(value.revision) &&
    (value.revision as number) > 0 &&
    (value.status === "submitted" ||
      value.status === "executing" ||
      value.status === "completed") &&
    typeof value.title === "string" &&
    typeof value.summary === "string" &&
    typeof value.bodyMarkdown === "string" &&
    isStringArray(value.assumptions) &&
    isStringArray(value.testPlan) &&
    typeof value.sourceSessionId === "string" &&
    typeof value.createdAt === "string" &&
    typeof value.updatedAt === "string" &&
    (value.lastExecutionError === undefined ||
      typeof value.lastExecutionError === "string")
  );
}

function normalizeStoredPlan(value: unknown): PlanArtifactV2 | undefined {
  if (isPlanArtifact(value)) return structuredClone(value);
  if (!isRecord(value)) return undefined;
  if (
    value.schemaVersion !== 1 ||
    typeof value.id !== "string" ||
    !Number.isInteger(value.revision) ||
    (value.status !== "ready" && value.status !== "executing") ||
    typeof value.title !== "string" ||
    typeof value.summary !== "string" ||
    typeof value.bodyMarkdown !== "string" ||
    !isStringArray(value.assumptions) ||
    !isStringArray(value.testPlan) ||
    typeof value.sourceSessionId !== "string" ||
    typeof value.createdAt !== "string" ||
    typeof value.updatedAt !== "string"
  ) {
    return undefined;
  }
  return {
    schemaVersion: 2,
    id: value.id,
    revision: value.revision as number,
    status: value.status === "executing" ? "executing" : "submitted",
    title: value.title,
    summary: value.summary,
    bodyMarkdown: value.bodyMarkdown,
    assumptions: [...value.assumptions],
    testPlan: [...value.testPlan],
    sourceSessionId: value.sourceSessionId,
    createdAt: value.createdAt,
    updatedAt: value.updatedAt,
  };
}

function normalizePlanContent(content: PlanContent): PlanContent {
  const title = content.title.trim();
  const summary = content.summary.trim();
  const bodyMarkdown = content.bodyMarkdown.trim();
  if (!title || !summary || !bodyMarkdown) {
    throw new Error("Plan title, summary, and bodyMarkdown must not be empty");
  }
  return {
    title,
    summary,
    bodyMarkdown,
    assumptions: normalizeList(content.assumptions),
    testPlan: normalizeList(content.testPlan),
  };
}

function normalizeList(items: string[]): string[] {
  return items.map((item) => item.trim()).filter(Boolean);
}

function isStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((item) => typeof item === "string");
}
