import {
  isJsonObject,
  requireString,
  requireStringArray,
  type JsonObject,
} from "./validation.ts";

export function parseSubagentOutput(
  text: string,
): JsonObject {
  const value = parseObject(text);
  validateTaskResult(value);
  return value;
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

function validateTaskResult(value: JsonObject): void {
  if (
    value.status !== "completed" &&
    value.status !== "failed" &&
    value.status !== "blocked"
  ) {
    throw new Error(
      "task_result.status must be completed, failed, or blocked",
    );
  }
  requireString(value, "summary", "task_result");
  requireStringArray(value, "evidence", "task_result");
  requireStringArray(value, "changedPaths", "task_result");
  requireStringArray(value, "blockers", "task_result");
  if (!Array.isArray(value.verification)) {
    throw new Error("task_result.verification must be an array");
  }
  for (const [index, item] of value.verification.entries()) {
    if (!isJsonObject(item)) {
      throw new Error(`task_result.verification[${index}] must be an object`);
    }
    requireString(item, "command", `task_result.verification[${index}]`);
    if (item.exitCode !== null && typeof item.exitCode !== "number") {
      throw new Error(
        `task_result.verification[${index}].exitCode must be a number or null`,
      );
    }
    if (typeof item.output !== "string") {
      throw new Error(
        `task_result.verification[${index}].output must be a string`,
      );
    }
    if (
      item.fullOutputPath !== undefined &&
      typeof item.fullOutputPath !== "string"
    ) {
      throw new Error(
        `task_result.verification[${index}].fullOutputPath must be a string`,
      );
    }
  }
}
