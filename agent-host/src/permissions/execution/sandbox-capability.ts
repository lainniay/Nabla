export type SandboxMode = "enforced" | "degraded" | "disabled";
export type SandboxBackendKind = "bubblewrap" | "seatbelt" | "none";

export interface SandboxCapability {
  mode: SandboxMode;
  backend: SandboxBackendKind;
  reason?: string;
  supportsFilesystemIsolation: boolean;
  supportsNetworkIsolation: boolean;
}

export function disabledSandbox(reason: string): SandboxCapability {
  return {
    mode: "disabled",
    backend: "none",
    reason,
    supportsFilesystemIsolation: false,
    supportsNetworkIsolation: false,
  };
}
