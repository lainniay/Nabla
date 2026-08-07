import { isCredentialPath } from "../filesystem/credential.ts";
import type { PermissionIntent, PermissionRule } from "../model.ts";
import type { ShellAnalysis } from "../shell/planner.ts";

export function buildCredentialDenyRules(
  intent: PermissionIntent,
): PermissionRule[] {
  const rules: PermissionRule[] = [];
  for (const atom of intent.atoms) {
    if (
      atom.kind === "file" &&
      (atom.operation === "read" || atom.operation === "list") &&
      isCredentialPath(atom.path)
    ) {
      rules.push({
        id: `builtin-credential-deny-${atom.operation}-${atom.path}`,
        effect: "deny",
        source: "builtin",
        matcher: {
          kind: "file",
          operation: atom.operation,
          path: atom.path,
        },
      });
    }
  }
  return rules;
}

export function buildReadOnlyBashRules(
  shellAnalysis: ShellAnalysis | undefined,
  intent: PermissionIntent,
): PermissionRule[] {
  if (!shellAnalysis?.safety.readOnly) return [];
  const rules: PermissionRule[] = [];
  for (const atom of intent.atoms) {
    if (atom.kind === "exec") {
      rules.push({
        id: `builtin-readonly-bash-${rules.length}`,
        effect: "allow",
        source: "builtin",
        matcher: {
          kind: "exec",
          executable: atom.executable,
          argv: atom.argv,
          cwd: atom.cwd,
        },
      });
    } else if (atom.kind === "file" && atom.path === "/dev/null") {
      rules.push({
        id: `builtin-readonly-bash-${rules.length}`,
        effect: "allow",
        source: "builtin",
        matcher: {
          kind: "file",
          operation: atom.operation,
          path: atom.path,
        },
      });
    }
  }
  return rules;
}

export function buildSandboxBashRules(
  shellAnalysis: ShellAnalysis | undefined,
  intent: PermissionIntent,
  sandboxEnforced: boolean,
): PermissionRule[] {
  if (!sandboxEnforced) return [];
  if (!intent.atoms.some((atom) => atom.kind === "exec")) return [];
  if (
    shellAnalysis === undefined ||
    shellAnalysis.safety.opaque ||
    shellAnalysis.safety.network ||
    shellAnalysis.safety.destructive
  ) {
    return [];
  }
  const rules: PermissionRule[] = [];
  for (const atom of intent.atoms) {
    if (atom.kind === "exec") {
      rules.push({
        id: `builtin-sandbox-bash-${rules.length}`,
        effect: "allow",
        source: "builtin",
        matcher: {
          kind: "exec",
          executable: atom.executable,
          argv: atom.argv,
          cwd: atom.cwd,
        },
      });
    } else if (atom.kind === "file") {
      rules.push({
        id: `builtin-sandbox-bash-${rules.length}`,
        effect: "allow",
        source: "builtin",
        matcher: {
          kind: "file",
          operation: atom.operation,
          path: atom.path,
          ...(atom.destination === undefined
            ? {}
            : { destination: atom.destination }),
        },
      });
    }
  }
  return rules;
}
