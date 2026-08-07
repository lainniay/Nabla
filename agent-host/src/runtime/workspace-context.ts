import { existsSync, readdirSync } from "node:fs";
import { basename, join } from "node:path";

const TREE_MAX_DEPTH = 2;
const DIR_ENTRY_LIMIT = 20;
const TOKEN_BUDGET = 1600;
const APPROX_BYTES_PER_TOKEN = 4;
const NOISY_DIR_NAMES = new Set([
  ".git",
  ".next",
  ".pytest_cache",
  ".ruff_cache",
  "__pycache__",
  "build",
  "dist",
  "node_modules",
  "out",
  "target",
]);

export function buildWorkspaceContext(cwd: string): string {
  const lines = [
    `Current working directory: ${cwd}`,
    `Working directory name: ${basename(cwd)}`,
  ];
  const gitRoot = findGitRoot(cwd);
  if (gitRoot !== undefined && gitRoot !== cwd) {
    lines.push(`Git root: ${gitRoot}`);
  }
  const tree = renderTree(cwd);
  if (tree.length > 0) {
    lines.push("", "Working directory tree:", ...tree);
  }
  return truncateLines(lines);
}

function renderTree(root: string): string[] {
  const lines: string[] = [];
  collectTreeLines(root, 0, lines);
  return lines;
}

function collectTreeLines(
  directory: string,
  depth: number,
  lines: string[],
): void {
  if (depth >= TREE_MAX_DEPTH) return;
  let entries: Array<{ name: string; isDirectory: boolean }> = [];
  try {
    entries = readdirSync(directory, { withFileTypes: true })
      .filter((entry) => {
        const name = entry.name;
        return !name.startsWith(".") && !NOISY_DIR_NAMES.has(name);
      })
      .map((entry) => ({
        name: entry.name,
        isDirectory: entry.isDirectory(),
      }));
  } catch {
    return;
  }
  entries.sort(
    (left, right) =>
      Number(right.isDirectory) - Number(left.isDirectory) ||
      left.name.localeCompare(right.name),
  );
  for (const entry of entries.slice(0, DIR_ENTRY_LIMIT)) {
    lines.push(
      `${"  ".repeat(depth)}- ${entry.name}${entry.isDirectory ? "/" : ""}`,
    );
    if (entry.isDirectory) {
      collectTreeLines(join(directory, entry.name), depth + 1, lines);
    }
  }
  if (entries.length > DIR_ENTRY_LIMIT) {
    lines.push(
      `${"  ".repeat(depth)}- ... ${entries.length - DIR_ENTRY_LIMIT} more entries`,
    );
  }
}

function findGitRoot(cwd: string): string | undefined {
  let current = cwd;
  while (true) {
    if (existsSync(join(current, ".git"))) return current;
    const parent = join(current, "..");
    if (parent === current) return undefined;
    current = parent;
  }
}

function truncateLines(lines: string[]): string {
  const maxBytes = TOKEN_BUDGET * APPROX_BYTES_PER_TOKEN;
  const result: string[] = [];
  let bytes = 0;
  for (const line of lines) {
    const lineBytes = Buffer.byteLength(line, "utf8") + 1;
    if (result.length > 0 && bytes + lineBytes > maxBytes) {
      result.push("... (truncated)");
      break;
    }
    result.push(line);
    bytes += lineBytes;
  }
  return result.join("\n");
}
