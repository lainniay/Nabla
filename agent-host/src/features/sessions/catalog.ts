import { existsSync } from "node:fs";
import { randomUUID } from "node:crypto";

import {
  SessionManager,
  type SessionInfo,
} from "@earendil-works/pi-coding-agent";

export type SessionScope = "current" | "all";
export type SessionSortMode = "threaded" | "recent" | "relevance";

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

function sanitizeLine(value: string): string {
  return value.replace(/[\u0000-\u001f\u007f]+/gu, " ").trim();
}

function samePath(left: string, right?: string): boolean {
  return right !== undefined && left === right;
}
