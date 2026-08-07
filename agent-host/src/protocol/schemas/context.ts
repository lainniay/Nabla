import { Type, type Static } from "typebox";

const ContextUsageStateSchema = Type.Union([
  Type.Literal("actual"),
  Type.Literal("estimated"),
  Type.Literal("recalculating"),
]);

const ContextCategorySchema = Type.Union([
  Type.Literal("user"),
  Type.Literal("assistant"),
  Type.Literal("toolResult"),
  Type.Literal("other"),
]);

const PruneReasonSchema = Type.Union([
  Type.Literal("hard_limit"),
  Type.Literal("history_budget"),
  Type.Literal("superseded"),
]);

const CompactionReasonSchema = Type.Union([
  Type.Literal("manual"),
  Type.Literal("threshold"),
  Type.Literal("overflow"),
]);

export const ContextSnapshotSchema = Type.Object({
  scopeId: Type.Optional(Type.String()),
  revision: Type.Number(),
  usageState: ContextUsageStateSchema,
  actualTokens: Type.Union([Type.Null(), Type.Number()]),
  actualPercent: Type.Union([Type.Null(), Type.Number()]),
  contextWindow: Type.Union([Type.Null(), Type.Number()]),
  estimatedUnfilteredTokens: Type.Number(),
  estimatedNextRequestTokens: Type.Number(),
  categories: Type.Array(
    Type.Object({
      category: ContextCategorySchema,
      messageCount: Type.Number(),
      estimatedTokens: Type.Number(),
    }),
  ),
  estimatedSystemToolOtherTokens: Type.Union([Type.Null(), Type.Number()]),
  estimatedPrunedThisRequestTokens: Type.Number(),
  estimatedCurrentlyPrunableTokens: Type.Number(),
  estimatedCumulativeAvoidedTokens: Type.Number(),
  pruning: Type.Array(
    Type.Object({
      reason: PruneReasonSchema,
      count: Type.Number(),
      estimatedTokensSaved: Type.Number(),
    }),
  ),
  topConsumers: Type.Array(
    Type.Object({
      category: ContextCategorySchema,
      label: Type.String(),
      estimatedTokens: Type.Number(),
      toolCallId: Type.Optional(Type.String()),
    }),
  ),
  compactionCount: Type.Number(),
  recentCompactions: Type.Array(
    Type.Object({
      reason: CompactionReasonSchema,
      firstKeptEntryId: Type.String(),
      tokensBefore: Type.Number(),
      estimatedTokensAfter: Type.Union([Type.Null(), Type.Number()]),
      tokensSaved: Type.Union([Type.Null(), Type.Number()]),
      savedPercent: Type.Union([Type.Null(), Type.Number()]),
      fileCount: Type.Number(),
      readFileCount: Type.Number(),
      modifiedFileCount: Type.Number(),
    }),
  ),
  policy: Type.Object({
    enabled: Type.Boolean(),
    recentToolResultTokens: Type.Number(),
    minimumBatchSavingsTokens: Type.Number(),
    minimumToolResultTokens: Type.Number(),
    successToolResultLimitTokens: Type.Number(),
    searchToolResultLimitTokens: Type.Number(),
    errorToolResultLimitTokens: Type.Number(),
  }),
  epoch: Type.Number(),
});

export type ContextSnapshot = Static<typeof ContextSnapshotSchema>;
