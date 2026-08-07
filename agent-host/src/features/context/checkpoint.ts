import {
  PLAN_ENTRY_TYPE,
  planRevisionMarker,
  type PlanArtifact,
} from "../plans/model.ts";
import {
  compactionFileDetails,
  messageContentText,
} from "../../protocol/message-content.ts";
import { isJsonObject as isObject } from "../../protocol/validation.ts";
import type {
  AgentMessage,
  CompactionRecord,
  ContextActiveState,
} from "./model.ts";
import type { ContextBudgetManager } from "./engine.ts";

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

export function injectCheckpoint(
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

export function isPlanEntry(message: AgentMessage): boolean {
  return (
    isObject(message) &&
    message.role === "custom" &&
    message.customType === PLAN_ENTRY_TYPE
  );
}

function containsPlanRevision(
  message: AgentMessage,
  plan: PlanArtifact,
): boolean {
  if (!isObject(message)) return false;
  const candidates: unknown[] = [message.details];
  if (isObject(message.details)) {
    candidates.push(message.details.artifact);
  }
  for (const candidate of candidates) {
    if (
      isObject(candidate) &&
      candidate.id === plan.id &&
      candidate.revision === plan.revision
    ) {
      return true;
    }
  }
  const text = messageContentText(message.content);
  // ponytail: exact-token dedup treats a message that copies the marker without
  // the Plan body as "present"; acceptable because the token is namespaced and
  // only the implementation prompt emits it.
  return text.includes(planRevisionMarker(plan.id, plan.revision));
}

function messageRole(message: AgentMessage): string | undefined {
  return isObject(message) && typeof message.role === "string"
    ? message.role
    : undefined;
}
