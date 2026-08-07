import { basename } from "node:path";

import type { PermissionIntent } from "./model.ts";

const MUTATING_WORKTREE_ACTIONS = new Set([
  "add",
  "remove",
  "prune",
  "move",
  "repair",
  "lock",
  "unlock",
]);

export function mutatesManagedWorktree(intent: PermissionIntent): boolean {
  return intent.atoms.some((atom) => {
    if (atom.kind !== "exec") return false;
    let executable = basename(atom.executable);
    let argv = atom.argv;
    if (executable === "env") {
      const commandIndex = argv.findIndex((value) =>
        !value.startsWith("-") && !value.includes("="));
      if (commandIndex < 0) return false;
      executable = basename(argv[commandIndex]!);
      argv = argv.slice(commandIndex + 1);
    }
    if (executable !== "git") return false;
    const worktree = argv.indexOf("worktree");
    return worktree >= 0 &&
      MUTATING_WORKTREE_ACTIONS.has(argv[worktree + 1] ?? "");
  });
}
