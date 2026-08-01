import { realpath } from "node:fs/promises";
import { resolve } from "node:path";

import {
  isPathWithin,
  nearestExistingRealPath,
} from "./policy/path-boundary.ts";

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
