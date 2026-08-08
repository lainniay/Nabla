import assert from "node:assert/strict";
import test from "node:test";

import { TODO_ENTRY_TYPE, TodoStore } from "./store.ts";

function entry(data: unknown) {
  return { type: "custom", customType: TODO_ENTRY_TYPE, data };
}

test("replace normalizes items and enforces the one-in_progress rule", () => {
  const store = new TodoStore();
  assert.deepEqual(
    store.replace([
      { content: "  build  ", status: "in_progress" },
      { content: "test", status: "pending" },
      { content: "ship", status: "completed" },
    ]),
    {
      action: "created",
      todos: [
        { content: "build", status: "in_progress" },
        { content: "test", status: "pending" },
        { content: "ship", status: "completed" },
      ],
    },
  );
  assert.equal(
    store.replace([{ content: "next", status: "pending" }]).action,
    "updated",
  );
  assert.throws(
    () =>
      store.replace([
        { content: "a", status: "in_progress" },
        { content: "b", status: "in_progress" },
      ]),
    /at most one/u,
  );
  assert.throws(
    () => store.replace([{ content: "   ", status: "pending" }]),
    /content must be non-empty/u,
  );
  assert.deepEqual(store.replace([]), { action: "updated", todos: [] });
});

test("current and replace return copies that do not leak internal state", () => {
  const store = new TodoStore();
  const created = store.replace([{ content: "a", status: "pending" }]);
  created.todos[0]!.content = "mutated";
  assert.equal(store.current()[0]!.content, "a");
  const current = store.current();
  current[0]!.content = "mutated";
  assert.equal(store.current()[0]!.content, "a");
  const replaced = store.replace([{ content: "b", status: "pending" }]);
  replaced.todos[0]!.status = "completed";
  assert.equal(store.current()[0]!.status, "pending");
});

test("restore takes the last valid nabla.todo entry and ignores broken entries", () => {
  const store = new TodoStore();
  assert.deepEqual(
    store.onSessionActivated([
      entry("broken"),
      entry([
        { content: "a", status: "in_progress" },
        { content: "b", status: "in_progress" },
      ]),
      entry([{ content: "old", status: "completed" }]),
      entry([{ content: "new", status: "in_progress" }]),
    ]),
    [{ content: "new", status: "in_progress" }],
  );
  assert.deepEqual(store.onSessionActivated([]), []);
});
