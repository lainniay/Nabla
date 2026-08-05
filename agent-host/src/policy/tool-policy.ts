export const READ_ONLY_TOOL_NAMES = ["read", "grep", "find", "ls"] as const;
export const MUTATING_TOOL_NAMES = new Set(["edit", "write", "bash"]);

export const THINKING_LEVELS = [
  "off",
  "minimal",
  "low",
  "medium",
  "high",
  "xhigh",
  "max",
] as const;

export type ThinkingLevel = (typeof THINKING_LEVELS)[number];

/**
 * Advisory UI signal only. Permission decisions are made exclusively by the
 * structured permission kernel and never consult this function.
 */
export function isHighRiskCommand(command: string): boolean {
  return [
    /(^|\s)sudo(\s|$)/u,
    /(^|\s)rm\s+-(?:[^\s]*r[^\s]*f|[^\s]*f[^\s]*r)(\s|$)/u,
    /\bgit\s+reset\s+--hard\b/u,
    /\bgit\s+clean\s+-[^\s]*f/u,
    /\b(?:curl|wget)\b/u,
    /\b(?:chmod|chown)\b/u,
    /(?:^|\s)>(?:>?)\s*\/(?:etc|usr|bin|sbin)\//u,
  ].some((pattern) => pattern.test(command));
}
