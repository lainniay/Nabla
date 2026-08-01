import { realpath } from "node:fs/promises";
import { dirname, isAbsolute, relative, resolve, sep } from "node:path";

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
