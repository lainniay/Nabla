export type JsonObject = Record<string, unknown>;

export function stringArray(value: unknown): string[] {
  return Array.isArray(value)
    ? value.filter((item): item is string => typeof item === "string")
    : [];
}

export function isJsonObject(value: unknown): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
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
