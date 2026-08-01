import type {
  ContextEvent,
  ContextUsage,
} from "@earendil-works/pi-coding-agent";

import type { PlanArtifactV2 } from "./plan.ts";
import { MUTATING_TOOL_NAMES } from "./policy/tool-policy.ts";
import { isJsonObject as isObject } from "./protocol/validation.ts";
import {
  compactionFileDetails,
  messageContentText,
} from "./protocol/message-content.ts";

type AgentMessage = ContextEvent["messages"][number];
type JsonObject = Record<string, unknown>;

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
  plan?: PlanArtifactV2;
  goal?: Record<string, unknown>;
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

interface ToolCallInfo {
  id: string;
  name: string;
  normalizedName: string;
  arguments: JsonObject;
  canonicalArguments: string;
  sequence: number;
  mutationGeneration: number;
}

interface ToolResultInfo {
  index: number;
  message: ToolResultMessage;
  toolCall?: ToolCallInfo;
  originalTokens: number;
  currentTokens: number;
  hasImage: boolean;
}

interface ToolResultMessage extends JsonObject {
  role: "toolResult";
  toolCallId: string;
  toolName: string;
  content: Array<TextContent | ImageContent>;
  details?: unknown;
  isError: boolean;
}

interface TextContent extends JsonObject {
  type: "text";
  text: string;
}

interface ImageContent extends JsonObject {
  type: "image";
}

interface StickyDecision {
  reason: Exclude<PruneReason, "hard_limit">;
  placeholder: string;
}

interface Candidate {
  info: ToolResultInfo;
  reason: StickyDecision["reason"];
  placeholder: string;
  estimatedSavings: number;
}

interface MutablePruneStat {
  count: number;
  estimatedTokensSaved: number;
}

interface Checkpoint {
  key: string;
  message: AgentMessage;
}

const DEFAULT_POLICY: ContextPolicy = {
  enabled: true,
  recentToolResultTokens: 40_000,
  minimumBatchSavingsTokens: 20_000,
  minimumToolResultTokens: 50,
  successToolResultLimitTokens: 12_000,
  searchToolResultLimitTokens: 6_000,
  errorToolResultLimitTokens: 8_000,
};

const CHECKPOINT_TYPE = "nabla.context-checkpoint";
const PROTECTED_TOOL_NAMES = new Set([
  "ask_user",
  "submit_plan",
  "delegate_task",
]);
const SUPERSEDED_TOOL_NAMES = new Set(["read", "grep", "find", "ls"]);
const SEARCH_TOOL_NAMES = new Set([
  "grep",
  "find",
  "ls",
  "lsp",
  "ast",
  "search",
]);
const CATEGORY_ORDER: ContextCategoryEstimate["category"][] = [
  "user",
  "assistant",
  "toolResult",
  "other",
];
const PRUNE_REASON_ORDER: PruneReason[] = [
  "hard_limit",
  "history_budget",
  "superseded",
];

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
          messages,
          snapshot: this.snapshot(),
          applied: false,
        };
      }

      const filtered = structuredClone(messages);
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
    } catch (error) {
      this.warnOnce(
        "filter-fail-open",
        `Context pruning was skipped because the message view was not recognized: ${
          error instanceof Error ? error.message : String(error)
        }`,
      );
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
      ...(activeState.goal ? { goal: activeState.goal } : {}),
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
            goalId:
              typeof activeState.goal?.id === "string"
                ? activeState.goal.id
                : undefined,
            goalRevision:
              typeof activeState.goal?.revision === "number"
                ? activeState.goal.revision
                : undefined,
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

export function compactionRecordFromEntry(
  reason: CompactionRecord["reason"],
  entry: {
    firstKeptEntryId: string;
    tokensBefore: number;
    details?: unknown;
  },
  estimatedTokensAfter?: number,
): CompactionRecord {
  const after =
    estimatedTokensAfter !== undefined && estimatedTokensAfter >= 0
      ? estimatedTokensAfter
      : null;
  const saved =
    after === null ? null : Math.max(0, entry.tokensBefore - after);
  const { readFiles, modifiedFiles, fileCount } = compactionFileDetails(
    entry.details,
  );
  return {
    reason,
    firstKeptEntryId: entry.firstKeptEntryId,
    tokensBefore: entry.tokensBefore,
    estimatedTokensAfter: after,
    tokensSaved: saved,
    savedPercent:
      saved === null || entry.tokensBefore <= 0
        ? null
        : (saved / entry.tokensBefore) * 100,
    fileCount,
    readFileCount: readFiles.length,
    modifiedFileCount: modifiedFiles.length,
  };
}

function injectCheckpoint(
  messages: AgentMessage[],
  activeState: ContextActiveState,
  manager: ContextBudgetManager,
): void {
  let compactionIndex = -1;
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    if (messageRole(messages[index]) === "compactionSummary") {
      compactionIndex = index;
      break;
    }
  }
  if (compactionIndex < 0) return;

  const planAlreadyPresent =
    activeState.plan !== undefined &&
    messages
      .slice(compactionIndex + 1)
      .some((message) => containsPlanRevision(message, activeState.plan!));
  messages.splice(
    compactionIndex + 1,
    0,
    manager.checkpointFor(activeState, planAlreadyPresent),
  );
}

function containsPlanRevision(
  message: AgentMessage,
  plan: PlanArtifactV2,
): boolean {
  if (!isObject(message)) return false;
  const candidates: unknown[] = [message.details];
  if (isObject(message.details)) {
    candidates.push(message.details.artifact);
  }
  for (const candidate of candidates) {
    if (
      isObject(candidate) &&
      ((candidate.id === plan.id && candidate.revision === plan.revision) ||
        (candidate.planId === plan.id &&
          candidate.revision === plan.revision) ||
        (candidate.planId === plan.id &&
          candidate.planRevision === plan.revision))
    ) {
      return true;
    }
  }
  return false;
}

function collectToolCalls(messages: AgentMessage[]): Map<string, ToolCallInfo> {
  const calls = new Map<string, ToolCallInfo>();
  let sequence = 0;
  let mutationGeneration = 0;
  for (const message of messages) {
    if (!isObject(message) || message.role !== "assistant") continue;
    const content = Array.isArray(message.content) ? message.content : [];
    for (const part of content) {
      if (!isObject(part) || part.type !== "toolCall") continue;
      const name = typeof part.name === "string" ? part.name : "";
      const normalizedName = normalizeToolName(name);
      if (MUTATING_TOOL_NAMES.has(normalizedName)) {
        mutationGeneration += 1;
      }
      const argumentsValue = isObject(part.arguments) ? part.arguments : {};
      const id = typeof part.id === "string" ? part.id : "";
      calls.set(id, {
        id,
        name,
        normalizedName,
        arguments: argumentsValue,
        canonicalArguments: canonicalJson(normalizeArguments(argumentsValue)),
        sequence,
        mutationGeneration,
      });
      sequence += 1;
    }
  }
  return calls;
}

function collectToolResults(
  messages: AgentMessage[],
  calls: Map<string, ToolCallInfo>,
): ToolResultInfo[] {
  const results: ToolResultInfo[] = [];
  messages.forEach((message, index) => {
    if (!isToolResultMessage(message)) return;
    const textTokens = estimateTextTokens(toolResultText(message));
    results.push({
      index,
      message,
      toolCall: calls.get(message.toolCallId),
      originalTokens: textTokens,
      currentTokens: textTokens,
      hasImage: message.content.some((part) => part.type === "image"),
    });
  });
  return results;
}

function collectRecentProtected(
  results: ToolResultInfo[],
  protectedTokens: number,
): Set<string> {
  const protectedIds = new Set<string>();
  let tokens = 0;
  for (let index = results.length - 1; index >= 0; index -= 1) {
    if (tokens >= protectedTokens) break;
    const result = results[index];
    protectedIds.add(result.message.toolCallId);
    tokens += result.currentTokens;
  }
  return protectedIds;
}

function findSupersededResults(results: ToolResultInfo[]): Set<string> {
  const grouped = new Map<string, ToolResultInfo[]>();
  for (const result of results) {
    const call = result.toolCall;
    if (
      !call ||
      !SUPERSEDED_TOOL_NAMES.has(call.normalizedName) ||
      result.message.isError ||
      isNegativeEvidence(result)
    ) {
      continue;
    }
    const key = [
      call.normalizedName,
      call.canonicalArguments,
      call.mutationGeneration,
    ].join("\u0000");
    const group = grouped.get(key) ?? [];
    group.push(result);
    grouped.set(key, group);
  }

  const superseded = new Set<string>();
  for (const group of grouped.values()) {
    group.sort(
      (left, right) =>
        (left.toolCall?.sequence ?? 0) - (right.toolCall?.sequence ?? 0),
    );
    for (const result of group.slice(0, -1)) {
      superseded.add(result.message.toolCallId);
    }
  }
  return superseded;
}

function isNegativeEvidence(result: ToolResultInfo): boolean {
  const name = result.toolCall?.normalizedName;
  if (name !== "grep" && name !== "find" && name !== "ls") return false;
  const text = toolResultText(result.message).trim();
  if (text.length === 0) return true;
  return /(?:no\s+(?:matches|files|entries)|0\s+(?:matches|results)|empty\s+directory|not\s+found)/iu.test(
    text,
  );
}

function hardLimitFor(
  message: ToolResultMessage,
  policy: ContextPolicy,
): number {
  if (message.isError) return policy.errorToolResultLimitTokens;
  const name = normalizeToolName(message.toolName);
  return isSearchTool(name)
    ? policy.searchToolResultLimitTokens
    : policy.successToolResultLimitTokens;
}

function isSearchTool(name: string): boolean {
  return (
    SEARCH_TOOL_NAMES.has(name) ||
    name.includes("lsp") ||
    name.includes("ast") ||
    name.endsWith("_grep") ||
    name.endsWith("_find") ||
    name.endsWith("_search")
  );
}

function hardLimitMarker(info: ToolResultInfo, limit: number): string {
  return [
    "",
    `[Nabla hard-limited ${normalizeToolName(info.message.toolName)} result from ~${formatTokens(
      info.originalTokens,
    )} tokens to ${formatTokens(limit)} tokens.`,
    toolHint(info),
    "Re-run or re-read the tool if the omitted output is needed.]",
    "",
  ]
    .filter(Boolean)
    .join(" ");
}

function prunePlaceholder(
  info: ToolResultInfo,
  reason: StickyDecision["reason"],
): string {
  const reasonLabel =
    reason === "superseded"
      ? "superseded by a later identical successful call"
      : "outside the protected recent tool-result budget";
  return [
    `[Nabla pruned ${normalizeToolName(info.message.toolName)} result`,
    toolSummary(info.toolCall),
    `· original ~${formatTokens(info.originalTokens)} tokens`,
    `· reason: ${reasonLabel}.`,
    toolHint(info),
    "Re-run or re-read the tool if this evidence is needed.]",
  ]
    .filter(Boolean)
    .join(" ");
}

function toolHint(info: ToolResultInfo): string {
  const fullOutputPath = findFullOutputPath(info.message.details);
  return fullOutputPath ? `Pi full output: ${fullOutputPath}.` : "";
}

function findFullOutputPath(value: unknown): string | undefined {
  if (!isObject(value)) return undefined;
  if (typeof value.fullOutputPath === "string") return value.fullOutputPath;
  for (const child of Object.values(value)) {
    const found = findFullOutputPath(child);
    if (found) return found;
  }
  return undefined;
}

function toolSummary(call: ToolCallInfo | undefined): string {
  if (!call) return "";
  const args = call.arguments;
  const path = firstString(args, ["path", "file", "directory", "cwd"]);
  const query = firstString(args, ["pattern", "query", "glob", "symbol"]);
  const pieces = [path ? `· path ${safeSummary(path)}` : ""];
  if (query && call.normalizedName !== "read" && call.normalizedName !== "ls") {
    pieces.push(`· query ${safeSummary(query)}`);
  }
  return pieces.filter(Boolean).join(" ");
}

function truncateWithMarker(
  text: string,
  limitTokens: number,
  headRatio: number,
  marker: string,
): string {
  const characterBudget = Math.max(0, limitTokens * 4);
  if (text.length <= characterBudget) return text;
  const markerValue =
    marker.length <= characterBudget
      ? marker
      : safeSlice(marker, 0, characterBudget);
  const contentBudget = Math.max(0, characterBudget - markerValue.length);
  const headCharacters = Math.floor(contentBudget * headRatio);
  const tailCharacters = contentBudget - headCharacters;
  return [
    safeSlice(text, 0, headCharacters),
    markerValue,
    safeSlice(text, Math.max(0, text.length - tailCharacters), text.length),
  ].join("");
}

function safeSlice(value: string, start: number, end: number): string {
  let safeStart = Math.max(0, Math.min(value.length, start));
  let safeEnd = Math.max(safeStart, Math.min(value.length, end));
  if (
    safeStart > 0 &&
    safeStart < value.length &&
    isLowSurrogate(value.charCodeAt(safeStart))
  ) {
    safeStart += 1;
  }
  if (
    safeEnd > 0 &&
    safeEnd < value.length &&
    isHighSurrogate(value.charCodeAt(safeEnd - 1))
  ) {
    safeEnd -= 1;
  }
  return value.slice(safeStart, safeEnd);
}

function isHighSurrogate(code: number): boolean {
  return code >= 0xd800 && code <= 0xdbff;
}

function isLowSurrogate(code: number): boolean {
  return code >= 0xdc00 && code <= 0xdfff;
}

function estimateCategories(messages: AgentMessage[]): ContextCategoryEstimate[] {
  const categories = new Map<
    ContextCategoryEstimate["category"],
    ContextCategoryEstimate
  >(
    CATEGORY_ORDER.map((category) => [
      category,
      { category, messageCount: 0, estimatedTokens: 0 },
    ]),
  );
  for (const message of messages) {
    const category = messageCategory(message);
    const estimate = categories.get(category)!;
    estimate.messageCount += 1;
    estimate.estimatedTokens += estimateMessage(message);
  }
  return CATEGORY_ORDER.map((category) => ({ ...categories.get(category)! }));
}

function safeEstimateCategories(
  messages: AgentMessage[],
): ContextCategoryEstimate[] {
  try {
    return estimateCategories(messages);
  } catch {
    return CATEGORY_ORDER.map((category) => ({
      category,
      messageCount: 0,
      estimatedTokens: 0,
    }));
  }
}

function estimateTopConsumers(messages: AgentMessage[]): ContextConsumer[] {
  const calls = collectToolCalls(messages);
  return messages
    .map((message) => ({
      category: messageCategory(message),
      label: consumerLabel(message, calls),
      estimatedTokens: estimateMessage(message),
      ...(isToolResultMessage(message)
        ? { toolCallId: message.toolCallId }
        : {}),
    }))
    .sort(
      (left, right) => right.estimatedTokens - left.estimatedTokens,
    )
    .slice(0, 5);
}

function consumerLabel(
  message: AgentMessage,
  calls: Map<string, ToolCallInfo>,
): string {
  if (!isObject(message)) return "unknown message";
  switch (message.role) {
    case "user":
      return `user: ${safeSummary(messageContentText(message.content))}`;
    case "assistant":
      return "assistant response";
    case "toolResult": {
      const name =
        typeof message.toolName === "string" ? message.toolName : "unknown";
      const summary =
        typeof message.toolCallId === "string"
          ? toolSummary(calls.get(message.toolCallId))
          : "";
      return `${name} result ${summary}`.trim();
    }
    case "custom":
      return typeof message.customType === "string"
        ? message.customType
        : "custom state";
    case "compactionSummary":
      return "compaction summary";
    case "branchSummary":
      return "branch summary";
    case "bashExecution":
      return "bash execution";
    default:
      return "other context";
  }
}

function estimateMessages(messages: AgentMessage[]): number {
  return messages.reduce(
    (total, message) => total + estimateMessage(message),
    0,
  );
}

function safeEstimateMessages(messages: AgentMessage[]): number {
  try {
    return estimateMessages(messages);
  } catch {
    return 0;
  }
}

function estimateMessage(message: AgentMessage): number {
  if (!isObject(message)) return estimateTextTokens(String(message));
  switch (message.role) {
    case "user":
    case "custom":
      return estimateTextTokens(messageContentText(message.content));
    case "assistant":
      return estimateTextTokens(
        Array.isArray(message.content)
          ? message.content
              .map((part) => {
                if (!isObject(part)) return "";
                if (typeof part.text === "string") return part.text;
                if (part.type === "toolCall") {
                  return JSON.stringify({
                    name: part.name,
                    arguments: part.arguments,
                  });
                }
                return "";
              })
              .join("\n")
          : "",
      );
    case "toolResult":
      return isToolResultMessage(message)
        ? estimateTextTokens(toolResultText(message))
        : 0;
    case "bashExecution":
      return estimateTextTokens(
        [message.command, message.output].filter((value) => typeof value === "string").join("\n"),
      );
    case "branchSummary":
    case "compactionSummary":
      return estimateTextTokens(
        typeof message.summary === "string" ? message.summary : "",
      );
    default:
      return estimateTextTokens(JSON.stringify(message));
  }
}

function estimateTextTokens(value: string): number {
  return value.length === 0 ? 0 : Math.ceil(value.length / 4);
}

function toolResultText(message: ToolResultMessage): string {
  return message.content
    .filter((part): part is TextContent => part.type === "text")
    .map((part) => part.text)
    .join("\n");
}

function messageCategory(
  message: AgentMessage,
): ContextCategoryEstimate["category"] {
  const role = messageRole(message);
  if (role === "user") return "user";
  if (role === "assistant") return "assistant";
  if (role === "toolResult") return "toolResult";
  return "other";
}

function messageRole(message: AgentMessage): string | undefined {
  return isObject(message) && typeof message.role === "string"
    ? message.role
    : undefined;
}

function normalizeToolName(name: string): string {
  return name.trim().toLocaleLowerCase();
}

function normalizeArguments(value: unknown, key?: string): unknown {
  if (Array.isArray(value)) {
    return value.map((item) => normalizeArguments(item));
  }
  if (!isObject(value)) {
    if (
      typeof value === "string" &&
      key !== undefined &&
      ["path", "file", "directory", "cwd"].includes(key)
    ) {
      return normalizePath(value);
    }
    return value;
  }
  return Object.fromEntries(
    Object.entries(value)
      .sort(([left], [right]) => left.localeCompare(right))
      .map(([childKey, child]) => [
        childKey,
        normalizeArguments(child, childKey),
      ]),
  );
}

function normalizePath(value: string): string {
  const normalized = value.replaceAll("\\", "/").replace(/\/+/gu, "/");
  if (normalized === "/") return normalized;
  return normalized.replace(/\/$/u, "");
}

function canonicalJson(value: unknown): string {
  return JSON.stringify(value);
}

function firstString(object: JsonObject, keys: string[]): string | undefined {
  for (const key of keys) {
    const value = object[key];
    if (typeof value === "string" && value.length > 0) return value;
  }
  return undefined;
}

function safeSummary(value: string): string {
  const singleLine = value.replace(/\s+/gu, " ").trim();
  return singleLine.length <= 160
    ? singleLine
    : `${safeSlice(singleLine, 0, 157)}…`;
}

function formatTokens(tokens: number): string {
  if (tokens < 1_000) return String(tokens);
  if (tokens < 1_000_000) {
    return `${(tokens / 1_000).toFixed(tokens < 10_000 ? 1 : 0)}k`;
  }
  return `${(tokens / 1_000_000).toFixed(1)}m`;
}

function addPruneStat(stat: MutablePruneStat, savings: number): void {
  stat.count += 1;
  stat.estimatedTokensSaved += savings;
}

function totalPruned(
  pruning: Record<PruneReason, MutablePruneStat>,
): number {
  return PRUNE_REASON_ORDER.reduce(
    (total, reason) => total + pruning[reason].estimatedTokensSaved,
    0,
  );
}

function mutablePruning(): Record<PruneReason, MutablePruneStat> {
  return {
    hard_limit: { count: 0, estimatedTokensSaved: 0 },
    history_budget: { count: 0, estimatedTokensSaved: 0 },
    superseded: { count: 0, estimatedTokensSaved: 0 },
  };
}

function freezePruning(
  pruning: Record<PruneReason, MutablePruneStat>,
): ContextPruneEstimate[] {
  return PRUNE_REASON_ORDER.map((reason) => ({
    reason,
    ...pruning[reason],
  }));
}

function emptyPruning(): ContextPruneEstimate[] {
  return freezePruning(mutablePruning());
}

function emptySnapshot(policy: ContextPolicy): ContextSnapshot {
  return {
    revision: 0,
    usageState: "estimated",
    actualTokens: null,
    actualPercent: null,
    contextWindow: null,
    estimatedUnfilteredTokens: 0,
    estimatedNextRequestTokens: 0,
    categories: CATEGORY_ORDER.map((category) => ({
      category,
      messageCount: 0,
      estimatedTokens: 0,
    })),
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

function isToolResultMessage(
  message: AgentMessage | JsonObject,
): message is ToolResultMessage {
  if (
    !isObject(message) ||
    message.role !== "toolResult" ||
    typeof message.toolCallId !== "string" ||
    typeof message.toolName !== "string" ||
    typeof message.isError !== "boolean" ||
    !Array.isArray(message.content)
  ) {
    return false;
  }
  return message.content.every(
    (part) =>
      isObject(part) &&
      ((part.type === "text" && typeof part.text === "string") ||
        part.type === "image"),
  );
}
