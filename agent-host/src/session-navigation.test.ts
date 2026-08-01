import assert from "node:assert/strict";
import { mkdtempSync, realpathSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { SessionManager } from "@earendil-works/pi-coding-agent";

import {
  SessionCatalog,
  buildTreeSnapshot,
  copyTextForEntry,
  createStartupSessionManager,
  projectSessionHistory,
} from "./session-navigation.ts";

function user(text: string) {
  return {
    role: "user" as const,
    content: text,
    timestamp: Date.now(),
  };
}

function assistant(text: string) {
  return {
    role: "assistant" as const,
    content: [{ type: "text" as const, text }],
    api: "test",
    provider: "test",
    model: "test",
    usage: {
      input: 0,
      output: 0,
      cacheRead: 0,
      cacheWrite: 0,
      totalTokens: 0,
      cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
    },
    stopReason: "stop" as const,
    timestamp: Date.now(),
  };
}

test("startup creates a fresh session instead of continuing the latest plan", async () => {
  const root = realpathSync(
    mkdtempSync(join(tmpdir(), "nabla-session-startup-")),
  );
  try {
    const previous = SessionManager.create(root, root);
    previous.appendMessage(user("old plan"));
    previous.appendMessage(assistant("old implementation plan"));

    const startup = createStartupSessionManager(root, root);

    assert.notEqual(startup.getSessionId(), previous.getSessionId());
    assert.deepEqual(startup.getBranch(), []);
    const catalog = new SessionCatalog({ manager: startup });
    const resumable = await catalog.query("current", "old plan", "recent", false);
    assert.equal(resumable.sessions.length, 1);
    assert.equal(resumable.sessions[0]?.id, previous.getSessionId());
    assert.equal(resumable.sessions[0]?.current, false);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("session catalog searches full history but returns bounded summaries", async () => {
  const root = realpathSync(
    mkdtempSync(join(tmpdir(), "nabla-session-catalog-")),
  );
  try {
    const parent = SessionManager.create(root, root);
    parent.appendMessage(user("first session"));
    parent.appendMessage(assistant("contains hidden-search-needle"));
    const parentPath = parent.getSessionFile();
    assert.ok(parentPath);

    const child = SessionManager.create(root, root, {
      parentSession: parentPath,
    });
    child.appendMessage(user("child session"));
    child.appendMessage(assistant("child answer"));

    const catalog = new SessionCatalog({ manager: child });
    const searched = await catalog.query(
      "current",
      '"hidden-search-needle"',
      "relevance",
      false,
    );
    assert.equal(searched.sessions.length, 1);
    assert.equal(searched.sessions[0]?.id, parent.getSessionId());
    assert.equal("allMessagesText" in searched.sessions[0]!, false);

    const threaded = await catalog.query(
      "current",
      "",
      "threaded",
      false,
    );
    const childSummary = threaded.sessions.find(
      (session) => session.id === child.getSessionId(),
    );
    assert.equal(childSummary?.depth, 1);
    assert.equal(threaded.currentCwd, root);

    const bounded = await new SessionCatalog({
      manager: child,
      resultLimit: 1,
    }).query("current", "", "recent", false);
    assert.equal(bounded.sessions.length, 1);
    assert.equal(bounded.total, 2);
    assert.equal(bounded.truncated, true);
    assert.equal(bounded.offset, 0);
    assert.equal(bounded.nextOffset, 1);

    const secondPage = await new SessionCatalog({
      manager: child,
      resultLimit: 1,
    }).query("current", "", "recent", false, bounded.nextOffset ?? 0);
    assert.equal(secondPage.sessions.length, 1);
    assert.equal(secondPage.offset, 1);
    assert.equal(secondPage.nextOffset, null);
    assert.equal(secondPage.truncated, false);

    const searchedBeyondFirstPage = await new SessionCatalog({
      manager: child,
      resultLimit: 1,
    }).query("current", '"first session"', "relevance", false);
    assert.equal(searchedBeyondFirstPage.sessions[0]?.id, parent.getSessionId());
    assert.equal(searchedBeyondFirstPage.truncated, false);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("history projection follows Pi context entries and hides model-only custom messages", () => {
  const manager = SessionManager.inMemory("/tmp");
  const first = manager.appendMessage(user("hello"));
  manager.appendMessage(assistant("world"));
  manager.appendCustomMessageEntry("hidden", "secret checkpoint", false);
  manager.appendCustomMessageEntry("visible", "visible note", true);
  manager.appendCompaction("do not print full summary", first, 42_000, {
    readFiles: ["a.ts"],
    modifiedFiles: ["b.ts"],
  });

  const history = projectSessionHistory(manager.buildContextEntries());
  assert.ok(history.some((item) => item.kind === "user" && item.text === "hello"));
  assert.ok(
    history.some((item) => item.kind === "assistant" && item.text === "world"),
  );
  assert.ok(
    history.some((item) => item.kind === "notice" && item.text === "visible note"),
  );
  assert.ok(!JSON.stringify(history).includes("secret checkpoint"));
  assert.ok(!JSON.stringify(history).includes("do not print full summary"));
  assert.ok(
    history.some(
      (item) =>
        item.kind === "compaction" &&
        item.tokensBefore === 42_000 &&
        item.fileCount === 2,
    ),
  );
});

test("history projection preserves assistant text and tool-call ordering", () => {
  const history = projectSessionHistory([
    {
      type: "message",
      id: "entry-1",
      parentId: null,
      timestamp: "2026-01-01T00:00:00.000Z",
      message: {
        ...assistant(""),
        content: [
          { type: "text", text: "before" },
          {
            type: "toolCall",
            id: "tool-1",
            name: "read",
            arguments: { path: "src/lib.rs" },
          },
          { type: "text", text: "after" },
        ],
      },
    },
  ] as never);

  assert.deepEqual(
    history.map((item) => item.kind),
    ["assistant", "toolCall", "assistant"],
  );
  assert.deepEqual(history[0], {
    kind: "assistant",
    text: "before",
    thinking: "",
  });
  assert.deepEqual(history[2], {
    kind: "assistant",
    text: "after",
    thinking: "",
  });
});

test("tree snapshot keeps active path, filters, folding, labels, and copy text", () => {
  const manager = SessionManager.inMemory("/tmp");
  const root = manager.appendMessage(user("root prompt"));
  const firstAnswer = manager.appendMessage(assistant("first answer"));
  const abandoned = manager.appendMessage(user("abandoned branch"));
  const sideAnswer = manager.appendMessage(assistant("side answer"));
  manager.branch(firstAnswer);
  const active = manager.appendMessage(user("active branch"));
  manager.appendLabelChange(active, "checkpoint");
  const activeAnswer = manager.appendMessage(assistant("active answer"));

  const all = buildTreeSnapshot(manager, "default", "", []);
  assert.ok(all.items.some((item) => item.entryId === abandoned));
  assert.ok(
    all.items.some(
      (item) => item.entryId === active && item.isActivePath && item.label === "checkpoint",
    ),
  );
  assert.deepEqual(
    all.items
      .filter((item) => item.entryId === root || item.entryId === firstAnswer)
      .map((item) => ({
        id: item.entryId,
        visualDepth: item.visualDepth,
        showConnector: item.showConnector,
      })),
    [
      { id: root, visualDepth: 0, showConnector: false },
      { id: firstAnswer, visualDepth: 0, showConnector: false },
    ],
  );
  assert.ok(
    all.items
      .filter((item) => item.entryId === abandoned || item.entryId === active)
      .every((item) => item.visualDepth === 1 && item.showConnector),
  );
  assert.deepEqual(
    all.items
      .filter((item) => item.entryId === activeAnswer)
      .map((item) => ({
        visualDepth: item.visualDepth,
        showConnector: item.showConnector,
        gutterPositions: item.gutterPositions,
      })),
    [{ visualDepth: 2, showConnector: false, gutterPositions: [0] }],
  );
  assert.deepEqual(
    all.items
      .filter((item) => item.entryId === sideAnswer)
      .map((item) => item.gutterPositions),
    [[]],
  );

  const searched = buildTreeSnapshot(manager, "default", "abandoned", []);
  assert.deepEqual(
    searched.items.map((item) => item.entryId),
    [abandoned],
  );

  const users = buildTreeSnapshot(manager, "user-only", "", []);
  assert.ok(users.items.every((item) => item.role === "user"));

  const folded = buildTreeSnapshot(manager, "all", "", [firstAnswer]);
  assert.ok(!folded.items.some((item) => item.entryId === active));
  assert.equal(
    copyTextForEntry(manager.getEntry(active)!),
    "active branch",
  );
});

test("tree visual layout keeps a long single-child chain flat", () => {
  const manager = SessionManager.inMemory("/tmp");
  for (let index = 0; index < 120; index += 1) {
    manager.appendMessage(user(`linear entry ${index}`));
  }

  const snapshot = buildTreeSnapshot(manager, "default", "", []);

  assert.equal(snapshot.items.length, 120);
  assert.ok(snapshot.items.every((item) => item.visualDepth === 0));
  assert.ok(snapshot.items.every((item) => !item.showConnector));
  assert.ok(snapshot.items.every((item) => item.gutterPositions.length === 0));
});
