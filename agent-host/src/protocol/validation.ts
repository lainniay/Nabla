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

export function stringField(value: JsonObject, name: string): string {
  const field = value[name];
  if (typeof field !== "string" || field.length === 0) {
    throw new Error(`Missing string field: ${name}`);
  }
  return field;
}

export function optionalStringField(
  value: JsonObject,
  name: string,
): string | undefined {
  const field = value[name];
  return typeof field === "string" ? field : undefined;
}

export function optionalNonNegativeIntegerField(
  value: JsonObject,
  name: string,
): number | undefined {
  const field = value[name];
  if (field === undefined) return undefined;
  if (!Number.isInteger(field) || (field as number) < 0) {
    throw new Error(`Invalid non-negative integer field: ${name}`);
  }
  return field as number;
}

export function stringArrayField(value: JsonObject, name: string): string[] {
  const field = value[name];
  if (field === undefined) return [];
  if (!Array.isArray(field) || !field.every((item) => typeof item === "string")) {
    throw new Error(`Invalid string array field: ${name}`);
  }
  return field;
}

export function enumField<const T extends readonly string[]>(
  value: JsonObject,
  name: string,
  choices: T,
): T[number] {
  const field = stringField(value, name);
  if (!choices.includes(field)) {
    throw new Error(`Unsupported ${name}: ${field}`);
  }
  return field as T[number];
}
