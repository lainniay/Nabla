export interface SandboxConfig {
  writableRoots: string[];
  unixSockets: {
    allow: string[];
    deny: string[];
  };
}

export const EMPTY_SANDBOX_CONFIG: SandboxConfig = {
  writableRoots: [],
  unixSockets: {
    allow: [],
    deny: [],
  },
};
