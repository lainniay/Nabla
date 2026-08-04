import { existsSync } from "node:fs";
import { randomUUID } from "node:crypto";

import {
  SessionManager,
  type SessionEntry,
  type SessionInfo,
  type SessionTreeNode,
} from "@earendil-works/pi-coding-agent";
import {
  compactionFileDetails,
  displayMessageText,
  messageContentText,
} from "./protocol/message-content.ts";
import { isJsonObject as isRecord } from "./protocol/validation.ts";

export type SessionScope = "current" | "all";
export type SessionSortMode = "threaded" | "recent" | "relevance";
export type TreeFilterMode =
  | "default"
  | "no-tools"
  | "user-only"
  | "labeled-only"
  | "all";

export const TURN_METRICS_ENTRY_TYPE = "nabla.turn-metrics.v1";

export interface TurnMetrics {
  turnId: string;
  startedAt: string;
  endedAt: string;
  durationMs: number;
}

export interface SessionSummary {
  path: string;
  id: string;
  cwd: string;
  cwdAvailable: boolean;
  name?: string;
  parentSessionPath?: string;
  createdAt: string;
  modifiedAt: string;
  messageCount: number;
  firstMessage: string;
  depth: number;
  isLast: boolean;
  current: boolean;
}

export interface SessionBrowserSnapshot {
  browserId: string;
  currentCwd: string;
  scope: SessionScope;
  query: string;
  sortMode: SessionSortMode;
  namedOnly: boolean;
  sessions: SessionSummary[];
  total: number;
  offset: number;
  nextOffset: number | null;
  truncated: boolean;
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

export interface TreeItem {
  entryId: string;
  parentId: string | null;
  kind: string;
  role?: string;
  preview: string;
  label?: string;
  labelTimestamp?: string;
  visualDepth: number;
  showConnector: boolean;
  gutterPositions: number[];
  isLast: boolean;
  isActivePath: boolean;
  isLeaf: boolean;
  foldable: boolean;
  folded: boolean;
}

export interface TreeSnapshot {
  items: TreeItem[];
  leafId: string | null;
  filterMode: TreeFilterMode;
  query: string;
}

interface SessionCatalogOptions {
  manager: SessionManager;
  resultLimit?: number;
  onProgress?: (
    browserId: string,
    scope: SessionScope,
    loaded: number,
    total: number,
  ) => void;
}

export class SessionCatalog {
  readonly browserId = randomUUID();
  private readonly cwd: string;
  private readonly sessionDir: string;
  private readonly defaultSessionDir: boolean;
  private readonly currentSessionPath?: string;
  private readonly onProgress?: SessionCatalogOptions["onProgress"];
  private readonly resultLimit: number;
  private current?: SessionInfo[];
  private all?: SessionInfo[];

  constructor(options: SessionCatalogOptions) {
    this.cwd = options.manager.getCwd();
    this.sessionDir = options.manager.getSessionDir();
    this.defaultSessionDir = options.manager.usesDefaultSessionDir();
    this.currentSessionPath = options.manager.getSessionFile();
    this.onProgress = options.onProgress;
    this.resultLimit = Math.max(1, options.resultLimit ?? 200);
  }

  async query(
    scope: SessionScope,
    query: string,
    sortMode: SessionSortMode,
    namedOnly: boolean,
    offset = 0,
  ): Promise<SessionBrowserSnapshot> {
    const sessions = await this.load(scope);
    const filtered = filterSessions(sessions, query, namedOnly);
    const ordered =
      sortMode === "threaded" && query.trim().length === 0
        ? flattenSessionThreads(filtered)
        : sortSessions(filtered, query, sortMode).map((session, index, all) => ({
            session,
            depth: 0,
            isLast: index === all.length - 1,
          }));

    const pageOffset = Math.min(
      Math.max(0, Number.isInteger(offset) ? offset : 0),
      ordered.length,
    );
    const pageEnd = Math.min(pageOffset + this.resultLimit, ordered.length);
    return {
      browserId: this.browserId,
      currentCwd: this.cwd,
      scope,
      query,
      sortMode,
      namedOnly,
      total: ordered.length,
      offset: pageOffset,
      nextOffset: pageEnd < ordered.length ? pageEnd : null,
      sessions: ordered.slice(pageOffset, pageEnd).map(({ session, depth, isLast }) =>
        sessionSummary(
          session,
          depth,
          isLast,
          samePath(session.path, this.currentSessionPath),
        ),
      ),
      truncated: pageEnd < ordered.length,
    };
  }

  private async load(scope: SessionScope): Promise<SessionInfo[]> {
    if (scope === "current") {
      if (!this.current) {
        this.current = await SessionManager.list(
          this.cwd,
          this.sessionDir,
          (loaded, total) =>
            this.onProgress?.(
              this.browserId,
              "current",
              loaded,
              total,
            ),
        );
      }
      return this.current;
    }

    if (!this.all) {
      this.all = this.defaultSessionDir
        ? await SessionManager.listAll((loaded, total) =>
            this.onProgress?.(this.browserId, "all", loaded, total),
          )
        : await SessionManager.listAll(
            this.sessionDir,
            (loaded, total) =>
              this.onProgress?.(this.browserId, "all", loaded, total),
          );
    }
    return this.all;
  }
}

export function createStartupSessionManager(
  cwd: string,
  sessionDir?: string,
): SessionManager {
  return SessionManager.create(cwd, sessionDir);
}

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

export function buildTreeSnapshot(
  manager: SessionManager,
  filterMode: TreeFilterMode,
  query: string,
  foldedEntryIds: readonly string[],
): TreeSnapshot {
  const tree = manager.getTree();
  const leafId = manager.getLeafId();
  const activePath = new Set(
    manager.getBranch().map((entry) => entry.id),
  );
  const ordered = flattenTree(tree, activePath);
  const byId = new Map(
    ordered.map((node) => [node.entry.id, node]),
  );
  const searchTokens = query
    .toLocaleLowerCase()
    .split(/\s+/u)
    .filter(Boolean);
  const folded = new Set(foldedEntryIds);
  const visibleIds = new Set<string>();

  for (const node of ordered) {
    const entry = node.entry;
    if (!passesTreeFilter(node, filterMode, leafId)) continue;
    const searchable = treeSearchText(node).toLocaleLowerCase();
    if (!searchTokens.every((token) => searchable.includes(token))) continue;
    if (hasFoldedAncestor(entry, folded, byId)) continue;
    visibleIds.add(entry.id);
  }

  const visibleParent = new Map<string, string | null>();
  const visibleChildren = new Map<string | null, string[]>();
  visibleChildren.set(null, []);
  for (const node of ordered) {
    if (!visibleIds.has(node.entry.id)) continue;
    let parentId = node.entry.parentId;
    while (parentId && !visibleIds.has(parentId)) {
      parentId = byId.get(parentId)?.entry.parentId ?? null;
    }
    visibleParent.set(node.entry.id, parentId);
    const children = visibleChildren.get(parentId) ?? [];
    children.push(node.entry.id);
    visibleChildren.set(parentId, children);
  }

  const visualLayout = buildTreeVisualLayout(visibleChildren);

  const items = ordered
    .filter((node) => visibleIds.has(node.entry.id))
    .map((node) => {
      const id = node.entry.id;
      const parentId = visibleParent.get(id) ?? null;
      const siblings = visibleChildren.get(parentId) ?? [];
      const visibleDirectChildren = visibleChildren.get(id) ?? [];
      const layout = visualLayout.get(id) ?? {
        visualDepth: 0,
        showConnector: false,
        gutterPositions: [],
        isLast: siblings.at(-1) === id,
      };
      const foldable =
        visibleDirectChildren.length > 0 &&
        (parentId === null ||
          (visibleChildren.get(parentId)?.length ?? 0) > 1);
      return {
        entryId: id,
        parentId,
        kind: node.entry.type,
        role: entryRole(node.entry),
        preview: treePreview(node),
        label: node.label,
        labelTimestamp: node.labelTimestamp,
        visualDepth: layout.visualDepth,
        showConnector: layout.showConnector,
        gutterPositions: layout.gutterPositions,
        isLast: layout.isLast,
        isActivePath: activePath.has(id),
        isLeaf: leafId === id,
        foldable,
        folded: folded.has(id),
      } satisfies TreeItem;
    });

  return { items, leafId, filterMode, query };
}

export function copyTextForEntry(entry: SessionEntry): string | undefined {
  switch (entry.type) {
    case "message": {
      const message = entry.message as unknown;
      if (!isRecord(message)) return undefined;
      if (message.role === "bashExecution") {
        return stringValue(message.command) || undefined;
      }
      const text = messageContentText(message.content, {
        imageMarker: "[image]",
        includeThinking: true,
      });
      return (
        text ||
        (message.role === "assistant"
          ? stringValue(message.errorMessage)
          : undefined) ||
        undefined
      );
    }
    case "custom_message":
      return (
        messageContentText(entry.content, {
          imageMarker: "[image]",
          includeThinking: true,
        }) || undefined
      );
    case "compaction":
      return entry.summary;
    case "branch_summary":
      return entry.summary;
    case "session_info":
      return entry.name;
    default:
      return undefined;
  }
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

function sessionSummary(
  session: SessionInfo,
  depth: number,
  isLast: boolean,
  current: boolean,
): SessionSummary {
  return {
    path: session.path,
    id: session.id,
    cwd: session.cwd,
    cwdAvailable: session.cwd.length === 0 || existsSync(session.cwd),
    name: session.name,
    parentSessionPath: session.parentSessionPath,
    createdAt: session.created.toISOString(),
    modifiedAt: session.modified.toISOString(),
    messageCount: session.messageCount,
    firstMessage: sanitizeLine(session.firstMessage),
    depth,
    isLast,
    current,
  };
}

function filterSessions(
  sessions: readonly SessionInfo[],
  query: string,
  namedOnly: boolean,
): SessionInfo[] {
  const parsed = parseSearch(query);
  return sessions.filter((session) => {
    if (namedOnly && !session.name?.trim()) return false;
    const text = [
      session.id,
      session.name ?? "",
      session.allMessagesText,
      session.cwd,
    ].join(" ");
    return matchesSearch(text, parsed);
  });
}

function sortSessions(
  sessions: readonly SessionInfo[],
  query: string,
  mode: SessionSortMode,
): SessionInfo[] {
  if (mode === "recent" || query.trim().length === 0) {
    return [...sessions].sort(
      (left, right) => right.modified.getTime() - left.modified.getTime(),
    );
  }
  const parsed = parseSearch(query);
  return [...sessions].sort((left, right) => {
    const leftScore = searchScore(
      `${left.id} ${left.name ?? ""} ${left.allMessagesText} ${left.cwd}`,
      parsed,
    );
    const rightScore = searchScore(
      `${right.id} ${right.name ?? ""} ${right.allMessagesText} ${right.cwd}`,
      parsed,
    );
    return (
      leftScore - rightScore ||
      right.modified.getTime() - left.modified.getTime()
    );
  });
}

function flattenSessionThreads(
  sessions: readonly SessionInfo[],
): Array<{ session: SessionInfo; depth: number; isLast: boolean }> {
  const byPath = new Map(sessions.map((session) => [session.path, session]));
  const children = new Map<string | null, SessionInfo[]>();
  for (const session of sessions) {
    const parent =
      session.parentSessionPath && byPath.has(session.parentSessionPath)
        ? session.parentSessionPath
        : null;
    const list = children.get(parent) ?? [];
    list.push(session);
    children.set(parent, list);
  }
  for (const list of children.values()) {
    list.sort(
      (left, right) => right.modified.getTime() - left.modified.getTime(),
    );
  }
  const result: Array<{
    session: SessionInfo;
    depth: number;
    isLast: boolean;
  }> = [];
  const visit = (parent: string | null, depth: number): void => {
    const list = children.get(parent) ?? [];
    list.forEach((session, index) => {
      result.push({
        session,
        depth,
        isLast: index === list.length - 1,
      });
      visit(session.path, depth + 1);
    });
  };
  visit(null, 0);
  return result;
}

interface ParsedSearch {
  regex?: RegExp;
  invalidRegex: boolean;
  tokens: Array<{ value: string; phrase: boolean }>;
}

function parseSearch(query: string): ParsedSearch {
  const trimmed = query.trim();
  if (trimmed.startsWith("re:")) {
    try {
      return {
        regex: new RegExp(trimmed.slice(3).trim(), "iu"),
        invalidRegex: false,
        tokens: [],
      };
    } catch {
      return { invalidRegex: true, tokens: [] };
    }
  }
  const tokens: ParsedSearch["tokens"] = [];
  const matcher = /"([^"]+)"|(\S+)/gu;
  for (const match of trimmed.matchAll(matcher)) {
    const phrase = match[1] !== undefined;
    tokens.push({
      value: (match[1] ?? match[2] ?? "").toLocaleLowerCase(),
      phrase,
    });
  }
  return { invalidRegex: false, tokens };
}

function matchesSearch(text: string, parsed: ParsedSearch): boolean {
  if (parsed.invalidRegex) return false;
  if (parsed.regex) return parsed.regex.test(text);
  const normalized = text.toLocaleLowerCase();
  return parsed.tokens.every(({ value, phrase }) =>
    phrase
      ? normalized.includes(value)
      : fuzzySubsequenceScore(value, normalized) < Number.POSITIVE_INFINITY,
  );
}

function searchScore(text: string, parsed: ParsedSearch): number {
  if (parsed.invalidRegex) return Number.POSITIVE_INFINITY;
  if (parsed.regex) {
    const index = text.search(parsed.regex);
    return index < 0 ? Number.POSITIVE_INFINITY : index;
  }
  const normalized = text.toLocaleLowerCase();
  return parsed.tokens.reduce((total, { value, phrase }) => {
    if (phrase) {
      const index = normalized.indexOf(value);
      return total + (index < 0 ? 1_000_000 : index);
    }
    return total + fuzzySubsequenceScore(value, normalized);
  }, 0);
}

function fuzzySubsequenceScore(needle: string, haystack: string): number {
  if (!needle) return 0;
  let score = 0;
  let from = 0;
  let previous = -1;
  for (const character of needle) {
    const index = haystack.indexOf(character, from);
    if (index < 0) return Number.POSITIVE_INFINITY;
    score += previous < 0 ? index : index - previous - 1;
    previous = index;
    from = index + character.length;
  }
  return score;
}

function flattenTree(
  roots: readonly SessionTreeNode[],
  activePath: ReadonlySet<string>,
): SessionTreeNode[] {
  const result: SessionTreeNode[] = [];
  const visit = (node: SessionTreeNode): void => {
    result.push(node);
    const children = [...node.children].sort(
      (left, right) =>
        Number(activePath.has(right.entry.id)) -
        Number(activePath.has(left.entry.id)),
    );
    for (const child of children) visit(child);
  };
  const orderedRoots = [...roots].sort(
    (left, right) =>
      Number(activePath.has(right.entry.id)) -
      Number(activePath.has(left.entry.id)),
  );
  for (const root of orderedRoots) visit(root);
  return result;
}

interface TreeVisualLayout {
  visualDepth: number;
  showConnector: boolean;
  gutterPositions: number[];
  isLast: boolean;
}

function buildTreeVisualLayout(
  visibleChildren: ReadonlyMap<string | null, readonly string[]>,
): Map<string, TreeVisualLayout> {
  const roots = visibleChildren.get(null) ?? [];
  const multipleRoots = roots.length > 1;
  const result = new Map<string, TreeVisualLayout>();
  const stack: Array<{
    id: string;
    indent: number;
    justBranched: boolean;
    showConnector: boolean;
    isLast: boolean;
    gutters: Array<{ position: number; show: boolean }>;
    virtualRootChild: boolean;
  }> = [];

  for (let index = roots.length - 1; index >= 0; index -= 1) {
    stack.push({
      id: roots[index]!,
      indent: multipleRoots ? 1 : 0,
      justBranched: multipleRoots,
      showConnector: multipleRoots,
      isLast: index === roots.length - 1,
      gutters: [],
      virtualRootChild: multipleRoots,
    });
  }

  while (stack.length > 0) {
    const current = stack.pop()!;
    const visualDepth = multipleRoots
      ? Math.max(0, current.indent - 1)
      : current.indent;
    const connectorDisplayed =
      current.showConnector && !current.virtualRootChild;
    result.set(current.id, {
      visualDepth,
      showConnector: connectorDisplayed,
      gutterPositions: current.gutters
        .filter((gutter) => gutter.show)
        .map((gutter) => gutter.position),
      isLast: current.isLast,
    });

    const children = visibleChildren.get(current.id) ?? [];
    const multipleChildren = children.length > 1;
    const childIndent = multipleChildren
      ? current.indent + 1
      : current.justBranched && current.indent > 0
        ? current.indent + 1
        : current.indent;
    const connectorPosition = Math.max(0, visualDepth - 1);
    const childGutters = connectorDisplayed
      ? [
          ...current.gutters,
          { position: connectorPosition, show: !current.isLast },
        ]
      : current.gutters;

    for (let index = children.length - 1; index >= 0; index -= 1) {
      stack.push({
        id: children[index]!,
        indent: childIndent,
        justBranched: multipleChildren,
        showConnector: multipleChildren,
        isLast: index === children.length - 1,
        gutters: childGutters,
        virtualRootChild: false,
      });
    }
  }

  return result;
}

function passesTreeFilter(
  node: SessionTreeNode,
  filterMode: TreeFilterMode,
  leafId: string | null,
): boolean {
  const entry = node.entry;
  const role = entryRole(entry);
  const settingsEntry = [
    "label",
    "custom",
    "model_change",
    "thinking_level_change",
    "session_info",
  ].includes(entry.type);
  const toolResult = entry.type === "message" && role === "toolResult";
  const assistantToolOnly =
    entry.type === "message" &&
    role === "assistant" &&
    entry.id !== leafId &&
    messageHasToolCallsOnly(entry.message);
  if (assistantToolOnly) return false;
  switch (filterMode) {
    case "user-only":
      return entry.type === "message" && role === "user";
    case "no-tools":
      return !settingsEntry && !toolResult;
    case "labeled-only":
      return node.label !== undefined;
    case "all":
      return true;
    default:
      return !settingsEntry;
  }
}

function hasFoldedAncestor(
  entry: SessionEntry,
  folded: ReadonlySet<string>,
  byId: ReadonlyMap<string, SessionTreeNode>,
): boolean {
  let parentId = entry.parentId;
  while (parentId) {
    if (folded.has(parentId)) return true;
    parentId = byId.get(parentId)?.entry.parentId ?? null;
  }
  return false;
}

function treeSearchText(node: SessionTreeNode): string {
  return [
    node.label ?? "",
    node.entry.type,
    entryRole(node.entry) ?? "",
    copyTextForEntry(node.entry) ?? "",
  ].join(" ");
}

function treePreview(node: SessionTreeNode): string {
  const entry = node.entry;
  if (entry.type === "message") {
    const content = copyTextForEntry(entry) ?? "";
    return truncatePreview(content);
  }
  if (entry.type === "compaction") {
    return `[compaction: ${Math.round(entry.tokensBefore / 1_000)}k tokens]`;
  }
  if (entry.type === "branch_summary") {
    return `[branch summary] ${truncatePreview(entry.summary)}`;
  }
  if (entry.type === "custom_message") {
    return `[${entry.customType}] ${truncatePreview(
      messageContentText(entry.content, {
        imageMarker: "[image]",
        includeThinking: true,
      }),
    )}`;
  }
  if (entry.type === "session_info") {
    return `[session name] ${entry.name ?? ""}`;
  }
  return `[${entry.type}]`;
}

function entryRole(entry: SessionEntry): string | undefined {
  if (entry.type !== "message") return undefined;
  const message = entry.message as unknown;
  return isRecord(message) ? stringValue(message.role) || undefined : undefined;
}

function messageHasToolCallsOnly(message: unknown): boolean {
  if (!isRecord(message) || !Array.isArray(message.content)) return false;
  let hasTool = false;
  for (const part of message.content) {
    if (!isRecord(part)) continue;
    if (part.type === "toolCall") hasTool = true;
    if (
      (part.type === "text" && stringValue(part.text).trim()) ||
      (part.type === "thinking" &&
        (stringValue(part.thinking) || stringValue(part.text)).trim())
    ) {
      return false;
    }
  }
  return hasTool;
}

function truncatePreview(value: string): string {
  const normalized = sanitizeLine(value);
  return normalized.length > 240 ? `${normalized.slice(0, 239)}…` : normalized;
}

function sanitizeLine(value: string): string {
  return value.replace(/[\u0000-\u001f\u007f]+/gu, " ").trim();
}

function stringValue(value: unknown): string {
  return typeof value === "string" ? value : "";
}

function samePath(left: string, right?: string): boolean {
  return right !== undefined && left === right;
}
