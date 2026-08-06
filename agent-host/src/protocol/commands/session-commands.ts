import {
  type CommandDefinition,
  requestObject,
} from "../command-definition.ts";
import {
  enumField,
  optionalNonNegativeIntegerField,
  optionalStringField,
  stringArrayField,
  stringField,
} from "../validation.ts";
import type { TreeFilterMode } from "../../session-navigation.ts";
import type { SessionBrowserSnapshot, TreeSnapshot } from "../../session-navigation.ts";
import type { ContextSnapshot } from "../../context-manager.ts";
import type { JsonObject } from "../validation.ts";

const FILTER_MODES = [
  "default",
  "no-tools",
  "user-only",
  "labeled-only",
  "all",
] as const;

export interface SessionCommandPort {
  contextState(): ContextSnapshot;
  clearQueue(): JsonObject;
  openSessionBrowser(): Promise<SessionBrowserSnapshot>;
  querySessionBrowser(input: {
    browserId: string;
    scope: "current" | "all";
    sortMode: "threaded" | "recent" | "relevance";
    query: string;
    namedOnly: boolean;
    offset: number;
  }): Promise<SessionBrowserSnapshot>;
  closeSessionBrowser(browserId: string): void;
  newSession(): Promise<{ cancelled: boolean; activation?: JsonObject }>;
  resumeSession(input: {
    sessionPath: string;
    cwdOverride?: string;
  }): Promise<{ cancelled: boolean; activation?: JsonObject }>;
  treeState(input: {
    filterMode: TreeFilterMode;
    query: string;
    foldedEntryIds: string[];
  }): TreeSnapshot;
  setTreeLabel(input: { entryId: string; label?: string }): void;
  copyTreeEntry(entryId: string): Promise<void>;
  navigateTree(input: {
    entryId: string;
    summarize: boolean;
    customInstructions?: string;
  }): Promise<JsonObject>;
  abortTreeNavigation(): void;
}

export function createSessionCommands(ops: SessionCommandPort): CommandDefinition<any>[] {
  return [
    {
      type: "context_state",
      lane: undefined,
      decode: requestObject,
      handle: () => ops.contextState(),
    },
    {
      type: "queue_clear",
      lane: "session",
      decode: requestObject,
      handle: () => ops.clearQueue(),
    },
    {
      type: "session_browser_open",
      lane: "session-browser",
      decode: requestObject,
      handle: () => ops.openSessionBrowser(),
    },
    {
      type: "session_browser_query",
      lane: "session-browser",
      decode: (value) => {
        const request = requestObject(value);
        return {
          browserId: stringField(request, "browserId"),
          scope: enumField(request, "scope", ["current", "all"] as const),
          sortMode: enumField(
            request,
            "sortMode",
            ["threaded", "recent", "relevance"] as const,
          ),
          query: optionalStringField(request, "query") ?? "",
          namedOnly: request.namedOnly === true,
          offset: optionalNonNegativeIntegerField(request, "offset") ?? 0,
        };
      },
      handle: (_context, request) => ops.querySessionBrowser(request),
    },
    {
      type: "session_browser_close",
      lane: "session-browser",
      decode: (value) => {
        const request = requestObject(value);
        return {
          browserId: stringField(request, "browserId"),
        };
      },
      handle: (_context, request) => ops.closeSessionBrowser(request.browserId),
    },
    {
      type: "session_new",
      lane: "session",
      decode: requestObject,
      handle: () => ops.newSession(),
    },
    {
      type: "session_resume",
      lane: "session",
      decode: (value) => {
        const request = requestObject(value);
        return {
          sessionPath: stringField(request, "sessionPath"),
          cwdOverride: optionalStringField(request, "cwdOverride"),
        };
      },
      handle: (_context, request) => ops.resumeSession(request),
    },
    {
      type: "tree_state",
      lane: undefined,
      decode: (value) => {
        const request = requestObject(value);
        return {
          filterMode: enumField(request, "filterMode", FILTER_MODES) as TreeFilterMode,
          query: optionalStringField(request, "query") ?? "",
          foldedEntryIds: stringArrayField(request, "foldedEntryIds"),
        };
      },
      handle: (_context, request) => ops.treeState(request),
    },
    {
      type: "tree_label",
      lane: "session",
      decode: (value) => {
        const request = requestObject(value);
        return {
          entryId: stringField(request, "entryId"),
          label: optionalStringField(request, "label")?.trim() || undefined,
        };
      },
      handle: (_context, request) => ops.setTreeLabel(request),
    },
    {
      type: "tree_copy",
      lane: undefined,
      decode: (value) => {
        const request = requestObject(value);
        return {
          entryId: stringField(request, "entryId"),
        };
      },
      handle: (_context, request) => ops.copyTreeEntry(request.entryId),
    },
    {
      type: "tree_navigate",
      lane: "session",
      decode: (value) => {
        const request = requestObject(value);
        return {
          entryId: stringField(request, "entryId"),
          summarize: request.summarize === true,
          customInstructions: optionalStringField(
            request,
            "customInstructions",
          ),
        };
      },
      handle: (_context, request) => ops.navigateTree(request),
    },
    {
      type: "tree_abort",
      lane: "session",
      decode: requestObject,
      handle: () => ops.abortTreeNavigation(),
    },
  ];
}
