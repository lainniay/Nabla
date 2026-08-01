export type JsonObject = Record<string, unknown>;

export function stringArray(value: unknown): string[] {
  return Array.isArray(value)
    ? value.filter((item): item is string => typeof item === "string")
    : [];
}

export function isJsonObject(value: unknown): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function requireString(
  value: JsonObject,
  name: string,
  context: string,
): string {
  const field = value[name];
  if (typeof field !== "string" || field.trim().length === 0) {
    throw new Error(`${context}.${name} must be a non-empty string`);
  }
  return field;
}

export function requireStringArray(
  value: JsonObject,
  name: string,
  context: string,
): string[] {
  const field = value[name];
  if (!Array.isArray(field) || !field.every((item) => typeof item === "string")) {
    throw new Error(`${context}.${name} must be an array of strings`);
  }
  return field;
}

export function requireObject(
  value: JsonObject,
  name: string,
  context: string,
): JsonObject {
  const field = value[name];
  if (!isJsonObject(field)) {
    throw new Error(`${context}.${name} must be an object`);
  }
  return field;
}

export function requireArray(
  value: JsonObject,
  name: string,
  context: string,
): unknown[] {
  const field = value[name];
  if (!Array.isArray(field)) {
    throw new Error(`${context}.${name} must be an array`);
  }
  return field;
}

export function requireBoolean(
  value: JsonObject,
  name: string,
  context: string,
): boolean {
  const field = value[name];
  if (typeof field !== "boolean") {
    throw new Error(`${context}.${name} must be a boolean`);
  }
  return field;
}

export function requireFiniteNumber(
  value: JsonObject,
  name: string,
  context: string,
): number {
  const field = value[name];
  if (typeof field !== "number" || !Number.isFinite(field)) {
    throw new Error(`${context}.${name} must be a finite number`);
  }
  return field;
}
