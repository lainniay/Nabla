import { Type, type Static } from "typebox";

export const PlanArtifactSchema = Type.Object({
  id: Type.String(),
  title: Type.String(),
  summary: Type.String(),
  bodyMarkdown: Type.String(),
  handoffMarkdown: Type.String(),
  sourceSessionId: Type.String(),
  createdAt: Type.String(),
  updatedAt: Type.String(),
  revision: Type.Number(),
  assumptions: Type.Array(Type.String()),
  testPlan: Type.Array(Type.String()),
});

export type PlanArtifact = Static<typeof PlanArtifactSchema>;
