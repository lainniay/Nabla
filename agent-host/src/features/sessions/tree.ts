import type {
  SessionEntry,
  SessionManager,
  SessionTreeNode,
} from "@earendil-works/pi-coding-agent";

import { messageContentText } from "../../protocol/message-content.ts";
import { isJsonObject as isRecord } from "../../protocol/validation.ts";

export type TreeFilterMode =
  | "default"
  | "no-tools"
  | "user-only"
  | "labeled-only"
  | "all";

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
