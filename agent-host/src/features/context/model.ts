import type { ContextEvent } from "@earendil-works/pi-coding-agent";

import type { PlanArtifact } from "../plans/model.ts";
import type {
  CompactionRecord,
  CompactionReason,
  ContextCategory,
  ContextCategoryEstimate,
  ContextConsumer,
  ContextPolicy,
  ContextPruneEstimate,
  ContextSnapshot,
  ContextUsageState,
  PruneReason,
} from "../../protocol/schemas/context.ts";

export type AgentMessage = ContextEvent["messages"][number];
export type JsonObject = Record<string, unknown>;

export type {
  CompactionRecord,
  CompactionReason,
  ContextCategory,
  ContextCategoryEstimate,
  ContextConsumer,
  ContextPolicy,
  ContextPruneEstimate,
  ContextSnapshot,
  ContextUsageState,
  PruneReason,
} from "../../protocol/schemas/context.ts";

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
