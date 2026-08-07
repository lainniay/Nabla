import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";

export function canonicalJson(value: unknown): string {
  if (value === null || typeof value !== "object") return JSON.stringify(value);
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(",")}]`;
  const object = value as Record<string, unknown>;
  return `{${Object.keys(object)
    .sort()
    .map((key) => `${JSON.stringify(key)}:${canonicalJson(object[key])}`)
    .join(",")}}`;
}

export function digestValue(value: unknown): string {
  return createHash("sha256").update(canonicalJson(value)).digest("hex");
}

export function sha256Hex(input: string | Uint8Array): string {
  return createHash("sha256").update(input).digest("hex");
}

export function npmScriptDigest(
  path: string,
  script: string,
): string | undefined {
  try {
    const parsed: unknown = JSON.parse(readFileSync(path, "utf8"));
    if (
      typeof parsed !== "object" ||
      parsed === null ||
      !("scripts" in parsed) ||
      typeof parsed.scripts !== "object" ||
      parsed.scripts === null
    ) {
      return undefined;
    }
    const value = (parsed.scripts as Record<string, unknown>)[script];
    return typeof value === "string" ? digestValue(value) : undefined;
  } catch {
    return undefined;
  }
}
