import { Type, type Static } from "typebox";

export const QuestionOptionSchema = Type.Object({
  id: Type.String(),
  label: Type.String(),
  description: Type.Optional(Type.String()),
});

export type QuestionOption = Static<typeof QuestionOptionSchema>;

export const PlanQuestionSchema = Type.Object({
  id: Type.String(),
  prompt: Type.String(),
  options: Type.Array(QuestionOptionSchema),
});

export type PlanQuestion = Static<typeof PlanQuestionSchema>;
