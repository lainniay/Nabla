import { Type } from "typebox";
import { Value } from "typebox/value";

import { isJsonObject, type JsonObject } from "./validation.ts";

const VerificationItemSchema = Type.Object({
  command: Type.String(),
  exitCode: Type.Union([Type.Null(), Type.Number()]),
  output: Type.String(),
  fullOutputPath: Type.Optional(Type.String()),
});

const TaskResultSchema = Type.Object({
  status: Type.Union([
    Type.Literal("completed"),
    Type.Literal("failed"),
    Type.Literal("blocked"),
  ]),
  summary: Type.String(),
  evidence: Type.Array(Type.String()),
  changedPaths: Type.Array(Type.String()),
  blockers: Type.Array(Type.String()),
  verification: Type.Array(VerificationItemSchema),
});

export function parseSubagentOutput(
  text: string,
): JsonObject {
  const value = parseObject(text);
  const errors = [...Value.Errors(TaskResultSchema, value)];
  if (errors.length > 0) {
    throw new Error(
      errors
        .map((error) =>
          `${(error as unknown as { path?: string }).path || "value"}: ${error.message}`,
        )
        .join("; "),
    );
  }
  return Value.Parse(TaskResultSchema, value) as JsonObject;
}

function parseObject(text: string): JsonObject {
  const trimmed = text.trim();
  if (!trimmed) throw new Error("Subagent returned empty structured output");
  const fenced = trimmed.match(/```(?:json)?\s*([\s\S]*?)```/iu)?.[1]?.trim();
  const candidates = [fenced, trimmed].filter(
    (candidate): candidate is string => Boolean(candidate),
  );
  const firstBrace = trimmed.indexOf("{");
  const lastBrace = trimmed.lastIndexOf("}");
  if (firstBrace >= 0 && lastBrace > firstBrace) {
    candidates.push(trimmed.slice(firstBrace, lastBrace + 1));
  }
  for (const candidate of [...new Set(candidates)]) {
    try {
      const parsed = JSON.parse(candidate) as unknown;
      if (isJsonObject(parsed)) return parsed;
    } catch {
      // A fenced or surrounding-text candidate may still contain the object.
    }
  }
  throw new Error("Subagent output is not a valid JSON object");
}
