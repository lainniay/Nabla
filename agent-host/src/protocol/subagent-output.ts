import {
  isJsonObject,
  requireString,
  requireStringArray,
  type JsonObject,
} from "./validation.ts";

export type SubagentOutputKind = "task" | "goal_spec" | "review";

export function parseSubagentOutput(
  text: string,
  kind: SubagentOutputKind,
): JsonObject {
  const value = parseObject(text);
  if (kind === "task") validateTaskResult(value);
  else if (kind === "goal_spec") validateGoalSpec(value);
  else validateReview(value);
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

function validateGoalSpec(value: JsonObject): void {
  requireString(value, "summary", "goal_spec");
  requireStringArray(value, "acceptanceCriteria", "goal_spec");
  requireStringArray(value, "allowedTools", "goal_spec");
  requireStringArray(value, "allowedPaths", "goal_spec");
  requireStringArray(value, "allowedCommands", "goal_spec");
  if (!Array.isArray(value.tasks) || value.tasks.length === 0) {
    throw new Error("goal_spec.tasks must be a non-empty array");
  }
  for (const [index, task] of value.tasks.entries()) {
    if (!isJsonObject(task)) {
      throw new Error(`goal_spec.tasks[${index}] must be an object`);
    }
    const context = `goal_spec.tasks[${index}]`;
    requireString(task, "id", context);
    requireString(task, "title", context);
    requireString(task, "description", context);
    if (task.profile !== undefined) requireString(task, "profile", context);
    for (const field of ["dependsOn", "allowedPaths", "acceptanceCriteria"]) {
      if (task[field] !== undefined) requireStringArray(task, field, context);
    }
  }
}

function validateReview(value: JsonObject): void {
  if (
    value.verdict !== "pass" &&
    value.verdict !== "changes_required" &&
    value.verdict !== "blocked"
  ) {
    throw new Error(
      "review_result.verdict must be pass, changes_required, or blocked",
    );
  }
  requireString(value, "summary", "review_result");
  if (!Array.isArray(value.findings)) {
    throw new Error("review_result.findings must be an array");
  }
  for (const [index, finding] of value.findings.entries()) {
    if (!isJsonObject(finding)) {
      throw new Error(`review_result.findings[${index}] must be an object`);
    }
    const context = `review_result.findings[${index}]`;
    if (!["critical", "high", "medium", "low"].includes(String(finding.severity))) {
      throw new Error(`${context}.severity is invalid`);
    }
    requireString(finding, "title", context);
    requireString(finding, "evidence", context);
    requireString(finding, "recommendation", context);
    if (finding.path !== undefined && typeof finding.path !== "string") {
      throw new Error(`${context}.path must be a string`);
    }
    if (finding.line !== undefined && typeof finding.line !== "number") {
      throw new Error(`${context}.line must be a number`);
    }
    if (finding.taskIds !== undefined) {
      requireStringArray(finding, "taskIds", context);
    }
    if (finding.paths !== undefined) {
      requireStringArray(finding, "paths", context);
    }
  }
}
