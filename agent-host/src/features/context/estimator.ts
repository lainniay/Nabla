import { isJsonObject as isObject } from "../../protocol/validation.ts";
import { messageContentText } from "../../protocol/message-content.ts";
import { MUTATING_TOOL_NAMES } from "../permissions/shell/rules.ts";
import {
  CATEGORY_ORDER,
  type AgentMessage,
  type ContextCategoryEstimate,
  type ContextConsumer,
  type JsonObject,
  type TextContent,
  type ToolCallInfo,
  type ToolResultMessage,
} from "./model.ts";

export function estimateTextTokens(value: string): number {
  return value.length === 0 ? 0 : Math.ceil(value.length / 4);
}

export function estimateCategories(
  messages: AgentMessage[],
): ContextCategoryEstimate[] {
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

export function safeEstimateCategories(
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

export function estimateTopConsumers(messages: AgentMessage[]): ContextConsumer[] {
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

export function estimateMessages(messages: AgentMessage[]): number {
  return messages.reduce(
    (total, message) => total + estimateMessage(message),
    0,
  );
}

export function safeEstimateMessages(messages: AgentMessage[]): number {
  try {
    return estimateMessages(messages);
  } catch {
    return 0;
  }
}

export function estimateMessage(message: AgentMessage): number {
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

export function toolResultText(message: ToolResultMessage): string {
  return message.content
    .filter((part): part is TextContent => part.type === "text")
    .map((part) => part.text)
    .join("\n");
}

export function messageCategory(
  message: AgentMessage,
): ContextCategoryEstimate["category"] {
  const role = messageRole(message);
  if (role === "user") return "user";
  if (role === "assistant") return "assistant";
  if (role === "toolResult") return "toolResult";
  return "other";
}

export function messageRole(message: AgentMessage): string | undefined {
  return isObject(message) && typeof message.role === "string"
    ? message.role
    : undefined;
}

export function normalizeToolName(name: string): string {
  return name.trim().toLocaleLowerCase();
}

export function normalizeArguments(value: unknown, key?: string): unknown {
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

export function canonicalJson(value: unknown): string {
  return JSON.stringify(value);
}

export function firstString(object: JsonObject, keys: string[]): string | undefined {
  for (const key of keys) {
    const value = object[key];
    if (typeof value === "string" && value.length > 0) return value;
  }
  return undefined;
}

export function safeSummary(value: string): string {
  const singleLine = value.replace(/\s+/gu, " ").trim();
  return singleLine.length <= 160
    ? singleLine
    : `${safeSlice(singleLine, 0, 157)}…`;
}

export function formatTokens(tokens: number): string {
  if (tokens < 1_000) return String(tokens);
  if (tokens < 1_000_000) {
    return `${(tokens / 1_000).toFixed(tokens < 10_000 ? 1 : 0)}k`;
  }
  return `${(tokens / 1_000_000).toFixed(1)}m`;
}

export function safeSlice(value: string, start: number, end: number): string {
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

export function toolSummary(call: ToolCallInfo | undefined): string {
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

export function collectToolCalls(messages: AgentMessage[]): Map<string, ToolCallInfo> {
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

export function isToolResultMessage(
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
