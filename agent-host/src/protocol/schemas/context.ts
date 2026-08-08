import { Type, type Static } from "typebox";

export const ContextUsageStateSchema = Type.Union([
  Type.Literal("actual"),
  Type.Literal("estimated"),
  Type.Literal("recalculating"),
]);

export type ContextUsageState = Static<typeof ContextUsageStateSchema>;

export const ContextCategorySchema = Type.Union([
  Type.Literal("user"),
  Type.Literal("assistant"),
  Type.Literal("toolResult"),
  Type.Literal("other"),
]);

export type ContextCategory = Static<typeof ContextCategorySchema>;

export const PruneReasonSchema = Type.Union([
  Type.Literal("hard_limit"),
  Type.Literal("history_budget"),
  Type.Literal("superseded"),
]);

export type PruneReason = Static<typeof PruneReasonSchema>;

export const CompactionReasonSchema = Type.Union([
  Type.Literal("manual"),
  Type.Literal("threshold"),
  Type.Literal("overflow"),
]);

export type CompactionReason = Static<typeof CompactionReasonSchema>;

export const ContextCategoryEstimateSchema = Type.Object({
  category: ContextCategorySchema,
  messageCount: Type.Number(),
  estimatedTokens: Type.Number(),
});

export type ContextCategoryEstimate = Static<
  typeof ContextCategoryEstimateSchema
>;

export const ContextConsumerSchema = Type.Object({
  category: ContextCategorySchema,
  label: Type.String(),
  estimatedTokens: Type.Number(),
  toolCallId: Type.Optional(Type.String()),
});

export type ContextConsumer = Static<typeof ContextConsumerSchema>;

export const ContextPruneEstimateSchema = Type.Object({
  reason: PruneReasonSchema,
  count: Type.Number(),
  estimatedTokensSaved: Type.Number(),
});

export type ContextPruneEstimate = Static<typeof ContextPruneEstimateSchema>;

export const CompactionRecordSchema = Type.Object({
  reason: CompactionReasonSchema,
  firstKeptEntryId: Type.String(),
  tokensBefore: Type.Number(),
  estimatedTokensAfter: Type.Union([Type.Null(), Type.Number()]),
  tokensSaved: Type.Union([Type.Null(), Type.Number()]),
  savedPercent: Type.Union([Type.Null(), Type.Number()]),
  fileCount: Type.Number(),
  readFileCount: Type.Number(),
  modifiedFileCount: Type.Number(),
});

export type CompactionRecord = Static<typeof CompactionRecordSchema>;

export const ContextPolicySchema = Type.Object({
  enabled: Type.Boolean(),
  recentToolResultTokens: Type.Number(),
  minimumBatchSavingsTokens: Type.Number(),
  minimumToolResultTokens: Type.Number(),
  successToolResultLimitTokens: Type.Number(),
  searchToolResultLimitTokens: Type.Number(),
  errorToolResultLimitTokens: Type.Number(),
});

export type ContextPolicy = Static<typeof ContextPolicySchema>;

export const ContextSnapshotSchema = Type.Object({
  scopeId: Type.Optional(Type.String()),
  revision: Type.Number(),
  usageState: ContextUsageStateSchema,
  actualTokens: Type.Union([Type.Null(), Type.Number()]),
  actualPercent: Type.Union([Type.Null(), Type.Number()]),
  contextWindow: Type.Union([Type.Null(), Type.Number()]),
  estimatedUnfilteredTokens: Type.Number(),
  estimatedNextRequestTokens: Type.Number(),
  categories: Type.Array(ContextCategoryEstimateSchema),
  estimatedSystemToolOtherTokens: Type.Union([Type.Null(), Type.Number()]),
  estimatedPrunedThisRequestTokens: Type.Number(),
  estimatedCurrentlyPrunableTokens: Type.Number(),
  estimatedCumulativeAvoidedTokens: Type.Number(),
  pruning: Type.Array(ContextPruneEstimateSchema),
  topConsumers: Type.Array(ContextConsumerSchema),
  compactionCount: Type.Number(),
  recentCompactions: Type.Array(CompactionRecordSchema),
  policy: ContextPolicySchema,
  epoch: Type.Number(),
});

export type ContextSnapshot = Static<typeof ContextSnapshotSchema>;
