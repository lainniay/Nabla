import { existsSync, realpathSync } from "node:fs";
import { realpath } from "node:fs/promises";
import { basename, dirname, isAbsolute, relative, resolve, sep } from "node:path";

export function isPathWithin(root: string, target: string): boolean {
  const child = relative(resolve(root), resolve(target));
  return (
    child === "" ||
    (child !== ".." && !child.startsWith(`..${sep}`) && !isAbsolute(child))
  );
}

export function assertWorkspaceRelativePath(path: string): void {
  if (path.length === 0) throw new Error("Path must not be empty");
  if (isAbsolute(path)) throw new Error(`Path must be relative: ${path}`);
  const normalized = resolve("/", path);
  if (!isPathWithin("/", normalized)) {
    throw new Error(`Path escapes its workspace: ${path}`);
  }
  const segments = path.split(/[\\/]/u);
  if (segments.includes("..")) {
    throw new Error(`Path escapes its workspace: ${path}`);
  }
}

export function workspaceRelativePath(root: string, target: string): string {
  const resolvedRoot = resolve(root);
  const resolvedTarget = resolve(target);
  if (!isPathWithin(resolvedRoot, resolvedTarget)) {
    throw new Error(`Path is outside the workspace: ${target}`);
  }
  return relative(resolvedRoot, resolvedTarget).split(sep).join("/");
}

export async function nearestExistingRealPath(path: string): Promise<string> {
  let candidate = resolve(path);
  while (true) {
    try {
      return await realpath(candidate);
    } catch (error) {
      const code =
        error && typeof error === "object" && "code" in error
          ? String(error.code)
          : "";
      if (code !== "ENOENT" && code !== "ENOTDIR") throw error;
      const parent = dirname(candidate);
      if (parent === candidate) throw error;
      candidate = parent;
    }
  }
}

export async function workspacePathError(
  cwd: string,
  path: string,
): Promise<string | undefined> {
  if (path.length === 0) return "Mutation tool did not provide a path";

  const lexicalWorkspace = resolve(cwd);
  const workspace = await realpath(lexicalWorkspace);
  const candidate = resolve(lexicalWorkspace, path);
  if (!isPathWithin(lexicalWorkspace, candidate)) {
    return `Refusing to write outside the workspace: ${path}`;
  }

  const existing = await nearestExistingRealPath(candidate);
  if (!isPathWithin(workspace, existing)) {
    return `Refusing to follow a path outside the workspace: ${path}`;
  }
  return undefined;
}

export function canonicalizePath(cwd: string, input: string): string {
  const absolute = resolve(cwd, input);
  let existing = absolute;
  const suffix: string[] = [];
  while (!existsSync(existing)) {
    const parent = dirname(existing);
    if (parent === existing) return absolute;
    suffix.unshift(basename(existing));
    existing = parent;
  }
  return resolve(realpathSync(existing), ...suffix);
}

export function canonicalPath(path: string): string {
  try {
    return realpathSync(path);
  } catch {
    return isAbsolute(path) ? resolve(path) : resolve(process.cwd(), path);
  }
}

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

export function patternMatches(pattern: string, value: string): boolean {
  if (pattern === "*") return true;
  const expression = pattern
    .replace(/[.+^${}()|[\]\\]/gu, "\\$&")
    .replace(/\*\*/gu, "\u0000")
    .replace(/\*/gu, ".*")
    .replace(/\u0000/gu, ".*")
    .replace(/\?/gu, ".");
  try {
    return new RegExp(`^${expression}$`, "u").test(value);
  } catch {
    return false;
  }
}
