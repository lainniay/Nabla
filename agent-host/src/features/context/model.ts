import type { ContextEvent } from "@earendil-works/pi-coding-agent";

import type { PlanArtifact } from "../plans/model.ts";

export type AgentMessage = ContextEvent["messages"][number];
export type JsonObject = Record<string, unknown>;

export type PruneReason = "hard_limit" | "history_budget" | "superseded";
export type ContextUsageState = "actual" | "estimated" | "recalculating";

export interface ContextPolicy {
  enabled: boolean;
  recentToolResultTokens: number;
  minimumBatchSavingsTokens: number;
  minimumToolResultTokens: number;
  successToolResultLimitTokens: number;
  searchToolResultLimitTokens: number;
  errorToolResultLimitTokens: number;
}

export interface ContextCategoryEstimate {
  category: "user" | "assistant" | "toolResult" | "other";
  messageCount: number;
  estimatedTokens: number;
}

export interface ContextConsumer {
  category: ContextCategoryEstimate["category"];
  label: string;
  estimatedTokens: number;
  toolCallId?: string;
}

export interface ContextPruneEstimate {
  reason: PruneReason;
  count: number;
  estimatedTokensSaved: number;
}

export interface CompactionRecord {
  reason: "manual" | "threshold" | "overflow";
  firstKeptEntryId: string;
  tokensBefore: number;
  estimatedTokensAfter: number | null;
  tokensSaved: number | null;
  savedPercent: number | null;
  fileCount: number;
  readFileCount: number;
  modifiedFileCount: number;
}

export interface ContextSnapshot {
  scopeId?: string;
  revision: number;
  usageState: ContextUsageState;
  actualTokens: number | null;
  actualPercent: number | null;
  contextWindow: number | null;
  estimatedUnfilteredTokens: number;
  estimatedNextRequestTokens: number;
  categories: ContextCategoryEstimate[];
  estimatedSystemToolOtherTokens: number | null;
  estimatedPrunedThisRequestTokens: number;
  estimatedCurrentlyPrunableTokens: number;
  estimatedCumulativeAvoidedTokens: number;
  pruning: ContextPruneEstimate[];
  topConsumers: ContextConsumer[];
  compactionCount: number;
  recentCompactions: CompactionRecord[];
  policy: ContextPolicy;
  epoch: number;
}

export interface ContextActiveState {
  planMode: boolean;
  plan?: PlanArtifact;
}

export interface ContextRemaining {
  usedTokens: number;
  usedPercent: number | null;
  remainingTokens: number | null;
  remainingPercent: number | null;
}

export interface ContextFilterResult {
  messages: AgentMessage[];
  snapshot: ContextSnapshot;
  applied: boolean;
}

export interface ContextBudgetManagerOptions {
  env?: NodeJS.ProcessEnv;
  policy?: Partial<ContextPolicy>;
  now?: () => number;
}

export interface ToolCallInfo {
  id: string;
  name: string;
  normalizedName: string;
  arguments: JsonObject;
  canonicalArguments: string;
  sequence: number;
  mutationGeneration: number;
}

export interface ToolResultInfo {
  index: number;
  message: ToolResultMessage;
  toolCall?: ToolCallInfo;
  originalTokens: number;
  currentTokens: number;
  hasImage: boolean;
}

export interface ToolResultMessage extends JsonObject {
  role: "toolResult";
  toolCallId: string;
  toolName: string;
  content: Array<TextContent | ImageContent>;
  details?: unknown;
  isError: boolean;
}

export interface TextContent extends JsonObject {
  type: "text";
  text: string;
}

export interface ImageContent extends JsonObject {
  type: "image";
}

export interface StickyDecision {
  reason: Exclude<PruneReason, "hard_limit">;
  placeholder: string;
}

export interface Candidate {
  info: ToolResultInfo;
  reason: StickyDecision["reason"];
  placeholder: string;
  estimatedSavings: number;
}

export interface MutablePruneStat {
  count: number;
  estimatedTokensSaved: number;
}

export interface Checkpoint {
  key: string;
  message: AgentMessage;
}

export const DEFAULT_POLICY: ContextPolicy = {
  enabled: true,
  recentToolResultTokens: 40_000,
  minimumBatchSavingsTokens: 20_000,
  minimumToolResultTokens: 50,
  successToolResultLimitTokens: 12_000,
  searchToolResultLimitTokens: 6_000,
  errorToolResultLimitTokens: 8_000,
};

export const CHECKPOINT_TYPE = "nabla.context-checkpoint";
export const PROTECTED_TOOL_NAMES = new Set([
  "ask_user",
  "submit_plan",
  "delegate_task",
]);
export const SUPERSEDED_TOOL_NAMES = new Set(["read", "grep", "find", "ls"]);
export const SEARCH_TOOL_NAMES = new Set([
  "grep",
  "find",
  "ls",
  "lsp",
  "ast",
  "search",
]);
export const CATEGORY_ORDER: ContextCategoryEstimate["category"][] = [
  "user",
  "assistant",
  "toolResult",
  "other",
];
export const PRUNE_REASON_ORDER: PruneReason[] = [
  "hard_limit",
  "history_budget",
  "superseded",
];
