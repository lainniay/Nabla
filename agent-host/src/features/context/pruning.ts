import { isJsonObject as isObject } from "../../protocol/validation.ts";
import {
  PRUNE_REASON_ORDER,
  SEARCH_TOOL_NAMES,
  SUPERSEDED_TOOL_NAMES,
  type AgentMessage,
  type ContextPolicy,
  type ContextPruneEstimate,
  type MutablePruneStat,
  type PruneReason,
  type StickyDecision,
  type ToolCallInfo,
  type ToolResultInfo,
  type ToolResultMessage,
} from "./model.ts";
import {
  estimateTextTokens,
  formatTokens,
  isToolResultMessage,
  normalizeToolName,
  safeSlice,
  toolResultText,
  toolSummary,
} from "./estimator.ts";

export function collectToolResults(
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

export function collectRecentProtected(
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

export function findSupersededResults(results: ToolResultInfo[]): Set<string> {
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

export function isNegativeEvidence(result: ToolResultInfo): boolean {
  const name = result.toolCall?.normalizedName;
  if (name !== "grep" && name !== "find" && name !== "ls") return false;
  const text = toolResultText(result.message).trim();
  if (text.length === 0) return true;
  return /(?:no\s+(?:matches|files|entries)|0\s+(?:matches|results)|empty\s+directory|not\s+found)/iu.test(
    text,
  );
}

export function hardLimitFor(
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

export function hardLimitMarker(info: ToolResultInfo, limit: number): string {
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

export function prunePlaceholder(
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

export function truncateWithMarker(
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

export function addPruneStat(stat: MutablePruneStat, savings: number): void {
  stat.count += 1;
  stat.estimatedTokensSaved += savings;
}

export function totalPruned(
  pruning: Record<PruneReason, MutablePruneStat>,
): number {
  return PRUNE_REASON_ORDER.reduce(
    (total, reason) => total + pruning[reason].estimatedTokensSaved,
    0,
  );
}

export function mutablePruning(): Record<PruneReason, MutablePruneStat> {
  return {
    hard_limit: { count: 0, estimatedTokensSaved: 0 },
    history_budget: { count: 0, estimatedTokensSaved: 0 },
    superseded: { count: 0, estimatedTokensSaved: 0 },
  };
}

export function freezePruning(
  pruning: Record<PruneReason, MutablePruneStat>,
): ContextPruneEstimate[] {
  return PRUNE_REASON_ORDER.map((reason) => ({
    reason,
    ...pruning[reason],
  }));
}

export function emptyPruning(): ContextPruneEstimate[] {
  return freezePruning(mutablePruning());
}
