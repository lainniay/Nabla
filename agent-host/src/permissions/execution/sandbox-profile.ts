import { homedir, tmpdir } from "node:os";
import { resolve, sep } from "node:path";

import type { PermissionIntent } from "../model.ts";
import type { SandboxCapability } from "./sandbox-capability.ts";

export interface SandboxExecutionProfile {
  mode: SandboxCapability["mode"];
  backend: "native" | "none";
  filesystem: {
    readWrite: string[];
    denyRead: string[];
    denyWrite: string[];
  };
  network: "blocked" | "allowed";
}

const CREDENTIAL_PATHS = [".ssh", ".aws", ".gnupg"].map((name) =>
  resolve(homedir(), name),
);

export function buildSandboxProfile(
  intent: PermissionIntent,
  cwd: string,
  capability: SandboxCapability,
): SandboxExecutionProfile {
  if (capability.mode !== "enforced") {
    return {
      mode: capability.mode,
      backend: "none",
      filesystem: {
        readWrite: [],
        denyRead: CREDENTIAL_PATHS,
        denyWrite: CREDENTIAL_PATHS,
      },
      network: "blocked",
    };
  }

  const readWrite = new Set([resolve(cwd), tmpdir()]);
  const denyRead = new Set(CREDENTIAL_PATHS);
  const denyWrite = new Set(CREDENTIAL_PATHS);
  let network: "blocked" | "allowed" = "blocked";

  for (const atom of intent.atoms) {
    if (atom.kind === "file" && atom.operation !== "read" && atom.operation !== "list") {
      const path = resolve(atom.path);
      const credential = CREDENTIAL_PATHS.some(
        (credentialPath) =>
          path === credentialPath || path.startsWith(`${credentialPath}${sep}`),
      );
      if (credential) {
        denyRead.add(path);
        denyWrite.add(path);
      } else {
        readWrite.add(path);
      }
    }
    if (atom.kind === "network" && atom.operation === "connect") {
      network = "allowed";
    }
  }

  return {
    mode: "enforced",
    backend: "native",
    filesystem: {
      readWrite: [...readWrite].filter((path) => !denyWrite.has(path)),
      denyRead: [...denyRead],
      denyWrite: [...denyWrite],
    },
    network,
  };
}
