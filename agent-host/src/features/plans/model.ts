import { Value } from "typebox/value";

import {
  PlanArtifactSchema,
  type PlanArtifact,
} from "../../protocol/schemas/plans.ts";

export type { PlanArtifact } from "../../protocol/schemas/plans.ts";

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

export interface PlanSessionEntry {
  type: string;
  customType?: string;
  data?: unknown;
}

export interface PlanModeEntry {
  active: boolean;
}

export function planImplementationPrompt(artifact: PlanArtifact): string {
  return [
    `Execute Nabla Plan ${artifact.id} revision ${artifact.revision} as a normal agent turn.`,
    `Nabla plan artifact marker: ${planRevisionMarker(artifact.id, artifact.revision)}`,
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

export function planRevisionMarker(id: string, revision: number): string {
  return `nabla-plan-artifact:${id}:${revision}`;
}

export function isPlanArtifact(value: unknown): value is PlanArtifact {
  const artifact = value as PlanArtifact;
  return (
    Value.Check(PlanArtifactSchema, value) &&
    artifact.id.length > 0 &&
    Number.isInteger(artifact.revision) &&
    artifact.revision > 0
  );
}
