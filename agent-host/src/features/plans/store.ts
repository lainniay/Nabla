import { randomUUID } from "node:crypto";

import { isJsonObject as isRecord } from "../../protocol/validation.ts";
import {
  PLAN_ENTRY_TYPE,
  PLAN_MODE_ENTRY_TYPE,
  isPlanArtifact,
  type PlanArtifact,
  type PlanContent,
  type PlanModeEntry,
  type PlanSessionEntry,
} from "./model.ts";

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
