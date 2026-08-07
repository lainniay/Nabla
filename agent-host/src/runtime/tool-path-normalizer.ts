import { isAbsolute, relative, resolve } from "node:path";

import { isPathWithin } from "../policy/path-boundary.ts";

const PATH_FIELDS = ["path", "destination"] as const;

export function normalizeToolInputPaths(
  input: Record<string, unknown>,
  cwd: string,
): void {
  for (const field of PATH_FIELDS) {
    const value = input[field];
    if (typeof value !== "string") continue;
    const normalized = normalizePath(value, cwd);
    if (normalized !== undefined) input[field] = normalized;
  }
}

export function normalizePath(
  value: string,
  cwd: string,
): string | undefined {
  if (!isAbsolute(value)) return undefined;
  const absolute = resolve(value);
  if (!isPathWithin(cwd, absolute)) return undefined;
  const result = relative(cwd, absolute);
  return result === "" ? "." : result;
}
