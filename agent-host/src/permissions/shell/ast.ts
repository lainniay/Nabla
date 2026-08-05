export type ShellConnector =
  | "sequence"
  | "and"
  | "or"
  | "pipe"
  | "pipe_both"
  | "background";

export interface ShellRedirection {
  fd?: number;
  operation: "read" | "write" | "append";
  target: string;
}

export interface ShellCommand {
  type: "command";
  argv: string[];
  assignments: Record<string, string>;
  redirections: ShellRedirection[];
  substitutions: ShellScript[];
  source: string;
  opaqueReason?: string;
}

export interface ShellGroup {
  type: "group";
  script: ShellScript;
  source: string;
}

export type ShellNode = ShellCommand | ShellGroup;

export interface ShellScript {
  nodes: ShellNode[];
  connectors: ShellConnector[];
  source: string;
  opaqueReason?: string;
}
