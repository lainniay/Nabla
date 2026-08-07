import type { ContextUsage } from "@earendil-works/pi-coding-agent";

import { isJsonObject as isObject } from "../../protocol/validation.ts";
import { CHECKPOINT_TYPE, DEFAULT_POLICY, PROTECTED_TOOL_NAMES } from "./model.ts";
import type {
  AgentMessage,
  Candidate,
  Checkpoint,
  CompactionRecord,
  ContextActiveState,
  ContextBudgetManagerOptions,
  ContextCategoryEstimate,
  ContextFilterResult,
  ContextPolicy,
  ContextPruneEstimate,
  ContextRemaining,
  ContextSnapshot,
  JsonObject,
  StickyDecision,
  ToolResultMessage,
} from "./model.ts";
import {
  estimateCategories,
  collectToolCalls,
  estimateMessages,
  estimateTextTokens,
  estimateTopConsumers,
  isToolResultMessage,
  normalizeToolName,
  safeEstimateCategories,
  safeEstimateMessages,
  toolResultText,
} from "./estimator.ts";
import {
  addPruneStat,
  collectRecentProtected,
  collectToolResults,
  emptyPruning,
  findSupersededResults,
  freezePruning,
  hardLimitFor,
  hardLimitMarker,
  mutablePruning,
  prunePlaceholder,
  totalPruned,
  truncateWithMarker,
} from "./pruning.ts";
import { injectCheckpoint, isPlanEntry } from "./checkpoint.ts";

export class ContextBudgetManager {
  private readonly policy: ContextPolicy;
  private readonly now: () => number;
  private readonly stickyDecisions = new Map<string, StickyDecision>();
  private readonly warningKeys = new Set<string>();
  private readonly pendingWarnings: string[] = [];
  private checkpoint?: Checkpoint;
  private sessionId?: string;
  private epoch = 0;
  private revision = 0;
  private cumulativeAvoidedTokens = 0;
  private compactionCount = 0;
  private recentCompactions: CompactionRecord[] = [];
  private alignedOverheadTokens: number | null = null;
  private lastRequestEstimate: number | null = null;
  private snapshotValue: ContextSnapshot;

  constructor(options: ContextBudgetManagerOptions = {}) {
    const parsed = readPolicy(options.env ?? process.env, (key, warning) =>
      this.warnOnce(key, warning),
    );
    this.policy = {
      ...parsed,
      ...options.policy,
    };
    this.now = options.now ?? Date.now;
    this.snapshotValue = emptySnapshot(this.policy);
  }

  filter(
    messages: AgentMessage[],
    contextUsage: ContextUsage | undefined,
    activeState: ContextActiveState,
  ): ContextFilterResult {
    this.revision += 1;
    try {
      validateMessages(messages);
      this.observeUsage(contextUsage);

      const unfilteredTokens = estimateMessages(messages);
      const categories = estimateCategories(messages);
      const topConsumers = estimateTopConsumers(messages);
      const withoutPlanEntries = messages.filter(
        (message) => !isPlanEntry(message),
      );

      if (!this.policy.enabled) {
        this.lastRequestEstimate = unfilteredTokens;
        this.snapshotValue = this.buildSnapshot({
          estimatedUnfilteredTokens: unfilteredTokens,
          estimatedNextRequestTokens: unfilteredTokens,
          categories,
          estimatedPrunedThisRequestTokens: 0,
          estimatedCurrentlyPrunableTokens: 0,
          pruning: emptyPruning(),
          topConsumers,
        });
        return {
          messages:
            withoutPlanEntries.length === messages.length
              ? messages
              : withoutPlanEntries,
          snapshot: this.snapshot(),
          applied: false,
        };
      }

      const filtered = structuredClone(withoutPlanEntries);
      const toolCalls = collectToolCalls(filtered);
      const pruneStats = mutablePruning();
      const toolResults = collectToolResults(filtered, toolCalls);

      for (const info of toolResults) {
        if (info.hasImage) continue;
        const limit = hardLimitFor(info.message, this.policy);
        if (info.currentTokens <= limit) continue;

        const text = toolResultText(info.message);
        const marker = hardLimitMarker(info, limit);
        const ratio = info.message.isError ? 0.4 : 0.6;
        const truncated = truncateWithMarker(text, limit, ratio, marker);
        info.message.content = [{ type: "text", text: truncated }];
        const afterTokens = estimateTextTokens(truncated);
        addPruneStat(
          pruneStats.hard_limit,
          Math.max(0, info.currentTokens - afterTokens),
        );
        info.currentTokens = afterTokens;
      }

      const recentProtected = collectRecentProtected(
        toolResults,
        this.policy.recentToolResultTokens,
      );
      const superseded = findSupersededResults(toolResults);
      const candidates: Candidate[] = [];

      for (const info of toolResults) {
        if (
          info.hasImage ||
          PROTECTED_TOOL_NAMES.has(normalizeToolName(info.message.toolName)) ||
          recentProtected.has(info.message.toolCallId) ||
          this.stickyDecisions.has(info.message.toolCallId) ||
          info.currentTokens < this.policy.minimumToolResultTokens
        ) {
          continue;
        }

        const reason = superseded.has(info.message.toolCallId)
          ? "superseded"
          : "history_budget";
        const placeholder = prunePlaceholder(info, reason);
        const estimatedSavings = Math.max(
          0,
          info.currentTokens - estimateTextTokens(placeholder),
        );
        if (estimatedSavings > 0) {
          candidates.push({ info, reason, placeholder, estimatedSavings });
        }
      }

      const candidateSavings = candidates.reduce(
        (total, candidate) => total + candidate.estimatedSavings,
        0,
      );
      const applyBatch =
        candidateSavings >= this.policy.minimumBatchSavingsTokens;
      if (applyBatch) {
        for (const candidate of candidates) {
          this.stickyDecisions.set(candidate.info.message.toolCallId, {
            reason: candidate.reason,
            placeholder: candidate.placeholder,
          });
        }
      }

      for (const info of toolResults) {
        if (info.hasImage) continue;
        const decision = this.stickyDecisions.get(info.message.toolCallId);
        if (!decision) continue;
        const beforeTokens = info.currentTokens;
        info.message.content = [{ type: "text", text: decision.placeholder }];
        const afterTokens = estimateTextTokens(decision.placeholder);
        addPruneStat(
          pruneStats[decision.reason],
          Math.max(0, beforeTokens - afterTokens),
        );
        info.currentTokens = afterTokens;
      }

      injectCheckpoint(filtered, activeState, this);

      const prunedThisRequest = totalPruned(pruneStats);
      this.cumulativeAvoidedTokens += prunedThisRequest;
      const nextRequestTokens = estimateMessages(filtered);
      this.lastRequestEstimate = nextRequestTokens;
      this.snapshotValue = this.buildSnapshot({
        estimatedUnfilteredTokens: unfilteredTokens,
        estimatedNextRequestTokens: nextRequestTokens,
        categories,
        estimatedPrunedThisRequestTokens: prunedThisRequest,
        estimatedCurrentlyPrunableTokens: applyBatch ? 0 : candidateSavings,
        pruning: freezePruning(pruneStats),
        topConsumers,
      });

      return {
        messages: filtered,
        snapshot: this.snapshot(),
        applied:
          prunedThisRequest > 0 ||
          filtered.length !== messages.length,
      };
    } catch {
      const estimatedUnfilteredTokens = safeEstimateMessages(messages);
      const categories = safeEstimateCategories(messages);
      this.lastRequestEstimate = estimatedUnfilteredTokens;
      this.snapshotValue = this.buildSnapshot({
        estimatedUnfilteredTokens,
        estimatedNextRequestTokens: estimatedUnfilteredTokens,
        categories,
        estimatedPrunedThisRequestTokens: 0,
        estimatedCurrentlyPrunableTokens: 0,
        pruning: emptyPruning(),
        topConsumers: [],
      });
      return {
        messages,
        snapshot: this.snapshot(),
        applied: false,
      };
    }
  }

  onSessionStart(sessionId?: string): ContextSnapshot {
    this.revision += 1;
    const changedSession =
      this.sessionId !== undefined &&
      sessionId !== undefined &&
      this.sessionId !== sessionId;
    this.sessionId = sessionId;
    this.startEpoch();
    this.alignedOverheadTokens = null;
    this.lastRequestEstimate = null;
    if (changedSession || this.epoch === 1) {
      this.cumulativeAvoidedTokens = 0;
      this.compactionCount = 0;
      this.recentCompactions = [];
      this.snapshotValue = emptySnapshot(this.policy);
      this.snapshotValue.epoch = this.epoch;
      this.snapshotValue.revision = this.revision;
    } else {
      this.snapshotValue = {
        ...this.snapshotValue,
        revision: this.revision,
        epoch: this.epoch,
        estimatedSystemToolOtherTokens: null,
        estimatedPrunedThisRequestTokens: 0,
        estimatedCurrentlyPrunableTokens: 0,
        pruning: emptyPruning(),
      };
    }
    return this.snapshot();
  }

  onCompaction(record: CompactionRecord): ContextSnapshot {
    this.revision += 1;
    this.startEpoch();
    this.compactionCount += 1;
    this.recentCompactions = [...this.recentCompactions, structuredClone(record)].slice(
      -5,
    );
    this.alignedOverheadTokens = null;
    this.lastRequestEstimate = null;
    this.snapshotValue = {
      ...this.snapshotValue,
      revision: this.revision,
      usageState: "recalculating",
      actualTokens: null,
      actualPercent: null,
      estimatedSystemToolOtherTokens: null,
      estimatedPrunedThisRequestTokens: 0,
      estimatedCurrentlyPrunableTokens: 0,
      pruning: emptyPruning(),
      compactionCount: this.compactionCount,
      recentCompactions: structuredClone(this.recentCompactions),
      epoch: this.epoch,
    };
    return this.snapshot();
  }

  onTreeNavigation(): ContextSnapshot {
    this.revision += 1;
    this.startEpoch();
    this.alignedOverheadTokens = null;
    this.lastRequestEstimate = null;
    this.snapshotValue = {
      ...this.snapshotValue,
      revision: this.revision,
      usageState: "recalculating",
      actualTokens: null,
      actualPercent: null,
      estimatedSystemToolOtherTokens: null,
      estimatedPrunedThisRequestTokens: 0,
      estimatedCurrentlyPrunableTokens: 0,
      pruning: emptyPruning(),
      epoch: this.epoch,
    };
    return this.snapshot();
  }

  onModelResponse(contextUsage: ContextUsage | undefined): ContextSnapshot {
    this.revision += 1;
    this.snapshotValue.revision = this.revision;
    if (contextUsage) {
      this.snapshotValue.contextWindow = contextUsage.contextWindow;
    }
    if (contextUsage?.tokens !== null && contextUsage?.tokens !== undefined) {
      this.snapshotValue.actualTokens = contextUsage.tokens;
      this.snapshotValue.actualPercent =
        contextUsage.percent ??
        (contextUsage.contextWindow > 0
          ? (contextUsage.tokens / contextUsage.contextWindow) * 100
          : null);
      this.snapshotValue.usageState = "actual";
      this.alignedOverheadTokens =
        this.lastRequestEstimate === null
          ? null
          : Math.max(0, contextUsage.tokens - this.lastRequestEstimate);
      this.snapshotValue.estimatedSystemToolOtherTokens =
        this.alignedOverheadTokens;
    }
    return this.snapshot();
  }

  snapshot(): ContextSnapshot {
    return structuredClone(this.snapshotValue);
  }

  takeWarning(): string | undefined {
    if (this.pendingWarnings.length === 0) return undefined;
    return this.pendingWarnings.splice(0).join(" ");
  }

  checkpointFor(
    activeState: ContextActiveState,
    planAlreadyPresent: boolean,
  ): AgentMessage {
    const plan = activeState.plan;
    const checkpointState = {
      planMode: activeState.planMode,
      ...(plan && !planAlreadyPresent ? { plan } : {}),
      ...(plan && planAlreadyPresent
        ? { planAlreadyPresent: { id: plan.id, revision: plan.revision } }
        : {}),
    };
    const key = JSON.stringify(checkpointState);
    if (this.checkpoint?.key !== key) {
      this.checkpoint = {
        key,
        message: {
          role: "custom",
          customType: CHECKPOINT_TYPE,
          content: [
            "Nabla context checkpoint (model-only runtime state; not persisted).",
            JSON.stringify(checkpointState),
          ].join("\n"),
          display: false,
          details: {
            epoch: this.epoch,
            planId: plan?.id,
            planRevision: plan?.revision,
          },
          timestamp: this.now(),
        } as AgentMessage,
      };
    }
    return structuredClone(this.checkpoint.message);
  }

  private startEpoch(): void {
    this.epoch += 1;
    this.stickyDecisions.clear();
    this.checkpoint = undefined;
  }

  private observeUsage(contextUsage: ContextUsage | undefined): void {
    if (!contextUsage) return;
    this.snapshotValue.contextWindow = contextUsage.contextWindow;
  }

  private buildSnapshot(
    request: Pick<
      ContextSnapshot,
      | "estimatedUnfilteredTokens"
      | "estimatedNextRequestTokens"
      | "categories"
      | "estimatedPrunedThisRequestTokens"
      | "estimatedCurrentlyPrunableTokens"
      | "pruning"
      | "topConsumers"
    >,
  ): ContextSnapshot {
    const actualAvailable =
      this.snapshotValue.actualTokens !== null &&
      this.snapshotValue.actualPercent !== null;
    const usageState =
      this.snapshotValue.usageState === "recalculating"
        ? "recalculating"
        : actualAvailable
          ? "actual"
          : "estimated";
    return {
      revision: this.revision,
      usageState,
      actualTokens: this.snapshotValue.actualTokens,
      actualPercent: this.snapshotValue.actualPercent,
      contextWindow: this.snapshotValue.contextWindow,
      ...request,
      estimatedSystemToolOtherTokens: this.alignedOverheadTokens,
      estimatedCumulativeAvoidedTokens: this.cumulativeAvoidedTokens,
      compactionCount: this.compactionCount,
      recentCompactions: structuredClone(this.recentCompactions),
      policy: { ...this.policy },
      epoch: this.epoch,
    };
  }

  private warnOnce(key: string, warning: string): void {
    if (this.warningKeys.has(key)) return;
    this.warningKeys.add(key);
    this.pendingWarnings.push(warning);
  }
}

export function contextRemaining(snapshot: ContextSnapshot): ContextRemaining {
  const usedTokens =
    snapshot.actualTokens ?? snapshot.estimatedNextRequestTokens;
  const usedPercent =
    snapshot.actualPercent ??
    (snapshot.contextWindow && snapshot.contextWindow > 0
      ? (usedTokens / snapshot.contextWindow) * 100
      : null);
  const remainingTokens =
    snapshot.contextWindow === null || snapshot.contextWindow <= 0
      ? null
      : Math.max(0, snapshot.contextWindow - usedTokens);
  const remainingPercent =
    usedPercent === null ? null : Math.max(0, 100 - usedPercent);
  return { usedTokens, usedPercent, remainingTokens, remainingPercent };
}

function emptySnapshot(policy: ContextPolicy): ContextSnapshot {
  const categories: ContextCategoryEstimate[] = [
    { category: "user", messageCount: 0, estimatedTokens: 0 },
    { category: "assistant", messageCount: 0, estimatedTokens: 0 },
    { category: "toolResult", messageCount: 0, estimatedTokens: 0 },
    { category: "other", messageCount: 0, estimatedTokens: 0 },
  ];
  return {
    revision: 0,
    usageState: "estimated",
    actualTokens: null,
    actualPercent: null,
    contextWindow: null,
    estimatedUnfilteredTokens: 0,
    estimatedNextRequestTokens: 0,
    categories,
    estimatedSystemToolOtherTokens: null,
    estimatedPrunedThisRequestTokens: 0,
    estimatedCurrentlyPrunableTokens: 0,
    estimatedCumulativeAvoidedTokens: 0,
    pruning: emptyPruning(),
    topConsumers: [],
    compactionCount: 0,
    recentCompactions: [],
    policy: { ...policy },
    epoch: 0,
  };
}

function readPolicy(
  env: NodeJS.ProcessEnv,
  warn: (key: string, warning: string) => void,
): ContextPolicy {
  return {
    ...DEFAULT_POLICY,
    enabled: parseBoolean(
      env.NABLA_CONTEXT_PRUNING,
      DEFAULT_POLICY.enabled,
      "NABLA_CONTEXT_PRUNING",
      warn,
    ),
    recentToolResultTokens: parseNonNegativeInteger(
      firstDefined(env, [
        "NABLA_CONTEXT_PROTECTED_TOKENS",
        "NABLA_CONTEXT_RECENT_TOOL_TOKENS",
        "NABLA_CONTEXT_PROTECT_TOKENS",
      ]),
      DEFAULT_POLICY.recentToolResultTokens,
      "NABLA_CONTEXT_PROTECTED_TOKENS",
      warn,
    ),
    minimumBatchSavingsTokens: parseNonNegativeInteger(
      firstDefined(env, [
        "NABLA_CONTEXT_MIN_PRUNE_TOKENS",
        "NABLA_CONTEXT_MIN_SAVINGS_TOKENS",
      ]),
      DEFAULT_POLICY.minimumBatchSavingsTokens,
      "NABLA_CONTEXT_MIN_PRUNE_TOKENS",
      warn,
    ),
  };
}

function firstDefined(
  env: NodeJS.ProcessEnv,
  names: string[],
): string | undefined {
  for (const name of names) {
    if (env[name] !== undefined) return env[name];
  }
  return undefined;
}

function parseBoolean(
  value: string | undefined,
  fallback: boolean,
  name: string,
  warn: (key: string, warning: string) => void,
): boolean {
  if (value === undefined) return fallback;
  switch (value.trim().toLocaleLowerCase()) {
    case "on":
    case "true":
    case "1":
      return true;
    case "off":
    case "false":
    case "0":
      return false;
    default:
      warn(
        `invalid-${name}`,
        `${name} must be on/off, true/false, or 1/0; using ${String(fallback)}.`,
      );
      return fallback;
  }
}

function parseNonNegativeInteger(
  value: string | undefined,
  fallback: number,
  name: string,
  warn: (key: string, warning: string) => void,
): number {
  if (value === undefined) return fallback;
  if (/^\d+$/u.test(value.trim())) {
    const parsed = Number(value.trim());
    if (Number.isSafeInteger(parsed)) return parsed;
  }
  warn(
    `invalid-${name}`,
    `${name} must be a non-negative integer; using ${fallback}.`,
  );
  return fallback;
}

function validateMessages(messages: AgentMessage[]): void {
  if (!Array.isArray(messages)) throw new Error("messages is not an array");
  for (const message of messages) {
    if (!isObject(message) || typeof message.role !== "string") {
      throw new Error("message has no role");
    }
    switch (message.role) {
      case "user":
        validateContent(message.content, true);
        break;
      case "assistant":
        if (!Array.isArray(message.content)) {
          throw new Error("assistant content is not an array");
        }
        for (const part of message.content) {
          if (
            !isObject(part) ||
            !["text", "thinking", "toolCall"].includes(String(part.type))
          ) {
            throw new Error("assistant content block is unknown");
          }
          if (
            (part.type === "text" || part.type === "thinking") &&
            typeof part.text !== "string"
          ) {
            throw new Error("assistant text block is invalid");
          }
          if (
            part.type === "toolCall" &&
            (typeof part.id !== "string" ||
              typeof part.name !== "string" ||
              !isObject(part.arguments))
          ) {
            throw new Error("assistant tool call is invalid");
          }
        }
        break;
      case "toolResult":
        if (!isToolResultMessage(message)) {
          throw new Error("tool result is invalid");
        }
        break;
      case "custom":
        if (
          typeof message.customType !== "string" ||
          typeof message.display !== "boolean"
        ) {
          throw new Error("custom message is invalid");
        }
        validateContent(message.content, true);
        break;
      case "bashExecution":
        if (
          typeof message.command !== "string" ||
          typeof message.output !== "string"
        ) {
          throw new Error("bash execution message is invalid");
        }
        break;
      case "branchSummary":
      case "compactionSummary":
        if (typeof message.summary !== "string") {
          throw new Error("summary message is invalid");
        }
        break;
      default:
        throw new Error(
          `unknown message role ${String((message as { role?: unknown }).role)}`,
        );
    }
  }
}

function validateContent(content: unknown, allowString: boolean): void {
  if (allowString && typeof content === "string") return;
  if (!Array.isArray(content)) throw new Error("message content is invalid");
  for (const part of content) {
    if (
      !isObject(part) ||
      (part.type !== "text" && part.type !== "image") ||
      (part.type === "text" && typeof part.text !== "string")
    ) {
      throw new Error("message content block is unknown");
    }
  }
}
