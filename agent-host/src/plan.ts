import { randomUUID } from "node:crypto";
import { isJsonObject as isRecord } from "./protocol/validation.ts";

export const PLAN_ENTRY_TYPE = "nabla.plan";
export const PLAN_MODE_ENTRY_TYPE = "nabla.plan-mode.v1";

export interface PlanContent {
  title: string;
  summary: string;
  bodyMarkdown: string;
  assumptions: string[];
  testPlan: string[];
  handoffMarkdown: string;
}

export interface PlanArtifact extends PlanContent {
  id: string;
  revision: number;
  sourceSessionId: string;
  createdAt: string;
  updatedAt: string;
}

export interface PlanSessionEntry {
  type: string;
  customType?: string;
  data?: unknown;
}

export interface PlanModeEntry {
  active: boolean;
}

interface PlanStoreOptions {
  createId?: () => string;
  now?: () => string;
}

export class PlanStore {
  private artifact?: PlanArtifact;
  private readonly createId: () => string;
  private readonly now: () => string;

  constructor(options: PlanStoreOptions = {}) {
    this.createId = options.createId ?? randomUUID;
    this.now = options.now ?? (() => new Date().toISOString());
  }

  private nextArtifactTimestamp(previous?: string): string {
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

  latest(): PlanArtifact | undefined {
    return this.artifact ? structuredClone(this.artifact) : undefined;
  }

  clear(): void {
    this.artifact = undefined;
  }

  restore(entries: readonly PlanSessionEntry[]): PlanArtifact | undefined {
    const candidates = entries
      .filter(
        (entry) =>
          entry.type === "custom" &&
          entry.customType === PLAN_ENTRY_TYPE,
      )
      .map((entry) => normalizeStoredPlan(entry.data))
      .filter((artifact): artifact is PlanArtifact => artifact !== undefined);
    const restored = candidates.at(-1);
    if (!restored) {
      this.artifact = undefined;
      return undefined;
    }
    this.artifact = structuredClone(restored);
    return this.latest();
  }

  adopt(artifact: PlanArtifact): void {
    if (!isPlanArtifact(artifact)) throw new Error("Invalid PlanArtifact");
    this.artifact = structuredClone(artifact);
  }

  submit(content: PlanContent, sourceSessionId: string): PlanArtifact {
    const normalized = normalizePlanContent(content);
    const previous = this.artifact;
    const timestamp = this.nextArtifactTimestamp(previous?.updatedAt);
    this.artifact = {
      id: previous?.id ?? this.createId(),
      revision: (previous?.revision ?? 0) + 1,
      ...normalized,
      sourceSessionId,
      createdAt: previous?.createdAt ?? timestamp,
      updatedAt: timestamp,
    };
    return this.latest() as PlanArtifact;
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

export function planImplementationPrompt(artifact: PlanArtifact): string {
  return [
    `Execute Nabla Plan ${artifact.id} revision ${artifact.revision} as a normal agent turn.`,
    "The plan below is the authoritative implementation request.",
    "Implement it completely, run proportionate verification, and report the outcome.",
    "",
    "## Source objective and handoff",
    artifact.handoffMarkdown,
    "",
    "## Approved plan",
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

export function isPlanArtifact(value: unknown): value is PlanArtifact {
  if (!isRecord(value)) return false;
  return (
    typeof value.id === "string" &&
    value.id.length > 0 &&
    Number.isInteger(value.revision) &&
    (value.revision as number) > 0 &&
    typeof value.title === "string" &&
    typeof value.summary === "string" &&
    typeof value.bodyMarkdown === "string" &&
    typeof value.handoffMarkdown === "string" &&
    isStringArray(value.assumptions) &&
    isStringArray(value.testPlan) &&
    typeof value.sourceSessionId === "string" &&
    typeof value.createdAt === "string" &&
    typeof value.updatedAt === "string"
  );
}

function normalizeStoredPlan(value: unknown): PlanArtifact | undefined {
  if (isPlanArtifact(value)) return structuredClone(value);
  return undefined;
}

function normalizePlanContent(content: PlanContent): PlanContent {
  const title = content.title.trim();
  const summary = content.summary.trim();
  const bodyMarkdown = content.bodyMarkdown.trim();
  const handoffMarkdown = content.handoffMarkdown.trim();
  if (!title || !summary || !bodyMarkdown || !handoffMarkdown) {
    throw new Error(
      "Plan title, summary, bodyMarkdown, and handoffMarkdown must not be empty",
    );
  }
  return {
    title,
    summary,
    bodyMarkdown,
    handoffMarkdown,
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
