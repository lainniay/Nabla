import type { SessionEntry } from "@earendil-works/pi-coding-agent";

import {
  compactionFileDetails,
  displayMessageText,
  messageContentText,
} from "../../protocol/message-content.ts";
import { isJsonObject as isRecord } from "../../protocol/validation.ts";

export const TURN_METRICS_ENTRY_TYPE = "nabla.turn-metrics.v1";

export interface TurnMetrics {
  turnId: string;
  startedAt: string;
  endedAt: string;
  durationMs: number;
}

export type SessionHistoryItem =
  | { kind: "user"; text: string }
  | { kind: "assistant"; text: string; thinking: string }
  | { kind: "toolCall"; id: string; name: string; args: unknown }
  | {
      kind: "toolResult";
      id: string;
      name: string;
      output: string;
      details?: unknown;
      isError: boolean;
    }
  | { kind: "notice"; text: string }
  | {
      kind: "compaction";
      firstKeptEntryId: string;
      tokensBefore: number;
      fileCount: number;
    }
  | {
      kind: "turnBoundary";
      turnId: string;
      startedAt: string;
      endedAt: string;
      durationMs: number;
      estimated: boolean;
    }
  | { kind: "branchSummary"; summary: string };

export function projectSessionHistory(
  entries: readonly SessionEntry[],
): SessionHistoryItem[] {
  const result: SessionHistoryItem[] = [];
  let legacyTurn:
    | {
        turnId: string;
        startedAt: string;
        endedAt?: string;
        insertAt?: number;
      }
    | undefined;
  const flushLegacyTurn = (): void => {
    if (!legacyTurn?.endedAt) {
      legacyTurn = undefined;
      return;
    }
    const startedAtMs = Date.parse(legacyTurn.startedAt);
    const endedAtMs = Date.parse(legacyTurn.endedAt);
    if (!Number.isFinite(startedAtMs) || !Number.isFinite(endedAtMs)) {
      legacyTurn = undefined;
      return;
    }
    const boundary: SessionHistoryItem = {
      kind: "turnBoundary",
      turnId: legacyTurn.turnId,
      startedAt: legacyTurn.startedAt,
      endedAt: legacyTurn.endedAt,
      durationMs: Math.max(0, endedAtMs - startedAtMs),
      estimated: true,
    };
    result.splice(legacyTurn.insertAt ?? result.length, 0, boundary);
    legacyTurn = undefined;
  };

  for (const entry of entries) {
    switch (entry.type) {
      case "message": {
        const role = isRecord(entry.message)
          ? stringValue(entry.message.role)
          : "";
        if (role === "user") {
          flushLegacyTurn();
          legacyTurn = {
            turnId: `legacy-${entry.id}`,
            startedAt: entry.timestamp,
          };
        }
        projectMessage(entry.message, result);
        if (
          legacyTurn &&
          (role === "assistant" ||
            role === "toolResult" ||
            role === "bashExecution")
        ) {
          legacyTurn.endedAt = entry.timestamp;
          legacyTurn.insertAt = result.length;
        }
        break;
      }
      case "custom": {
        if (entry.customType !== TURN_METRICS_ENTRY_TYPE) break;
        const metrics = parseTurnMetrics(entry.data);
        if (!metrics) break;
        legacyTurn = undefined;
        result.push({
          kind: "turnBoundary",
          ...metrics,
          estimated: false,
        });
        break;
      }
      case "custom_message":
        if (entry.display) {
          result.push({
            kind: "notice",
            text: messageContentText(entry.content, {
              imageMarker: "[image]",
              includeThinking: true,
            }),
          });
        }
        break;
      case "compaction": {
        const { fileCount } = compactionFileDetails(entry.details);
        result.push({
          kind: "compaction",
          firstKeptEntryId: entry.firstKeptEntryId,
          tokensBefore: entry.tokensBefore,
          fileCount,
        });
        break;
      }
      case "branch_summary":
        result.push({ kind: "branchSummary", summary: entry.summary });
        break;
      default:
        break;
    }
  }
  flushLegacyTurn();
  return result;
}

function parseTurnMetrics(value: unknown): TurnMetrics | undefined {
  if (!isRecord(value)) return undefined;
  const turnId = stringValue(value.turnId);
  const startedAt = stringValue(value.startedAt);
  const endedAt = stringValue(value.endedAt);
  const durationMs = value.durationMs;
  if (
    !turnId ||
    !startedAt ||
    !endedAt ||
    typeof durationMs !== "number" ||
    !Number.isFinite(durationMs) ||
    durationMs < 0
  ) {
    return undefined;
  }
  return {
    turnId,
    startedAt,
    endedAt,
    durationMs: Math.round(durationMs),
  };
}

function projectMessage(message: unknown, result: SessionHistoryItem[]): void {
  if (!isRecord(message)) return;
  const role = stringValue(message.role);
  if (role === "user") {
    result.push({
      kind: "user",
      text: displayMessageText(
        messageContentText(message.content, {
          imageMarker: "[image]",
          includeThinking: true,
        }),
      ),
    });
    return;
  }
  if (role === "assistant") {
    const content = Array.isArray(message.content) ? message.content : [];
    let text = "";
    let thinking = "";
    const flushAssistant = (): void => {
      if (text || thinking) {
        result.push({ kind: "assistant", text, thinking });
        text = "";
        thinking = "";
      }
    };
    for (const part of content) {
      if (!isRecord(part)) continue;
      if (part.type === "text") {
        text += stringValue(part.text);
      } else if (part.type === "thinking") {
        thinking += stringValue(part.thinking) || stringValue(part.text);
      } else if (part.type === "toolCall") {
        flushAssistant();
        result.push({
          kind: "toolCall",
          id: stringValue(part.id),
          name: stringValue(part.name) || "tool",
          args: isRecord(part.arguments) ? part.arguments : {},
        });
      }
    }
    flushAssistant();
    return;
  }
  if (role === "toolResult") {
    result.push({
      kind: "toolResult",
      id: stringValue(message.toolCallId),
      name: stringValue(message.toolName) || "tool",
      output: messageContentText(message.content, {
        imageMarker: "[image]",
        includeThinking: true,
      }),
      ...(message.details === undefined ? {} : { details: message.details }),
      isError: message.isError === true,
    });
    return;
  }
  if (role === "bashExecution") {
    const command = stringValue(message.command);
    result.push({
      kind: "toolCall",
      id: stringValue(message.id) || `bash-${result.length}`,
      name: "bash",
      args: { command },
    });
    result.push({
      kind: "toolResult",
      id: stringValue(message.id) || `bash-${result.length - 1}`,
      name: "bash",
      output: stringValue(message.output),
      isError: message.exitCode !== 0 && message.exitCode !== undefined,
    });
  }
}

function stringValue(value: unknown): string {
  return typeof value === "string" ? value : "";
}
