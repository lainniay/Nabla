import { isJsonObject } from "../../protocol/validation.ts";

export const TODO_ENTRY_TYPE = "nabla.todo";

export type TodoStatus = "pending" | "in_progress" | "completed";

export interface TodoItem {
  content: string;
  status: TodoStatus;
}

export type TodoAction = "created" | "updated";

export interface TodoReplaceResult {
  action: TodoAction;
  todos: TodoItem[];
}

export class TodoStore {
  private items: TodoItem[] = [];

  current(): TodoItem[] {
    return structuredClone(this.items);
  }

  replace(items: TodoItem[]): TodoReplaceResult {
    const normalized = parseTodoList(items);
    if (!normalized) {
      throw new Error(
        "Invalid todo list: content must be non-empty and at most one item may be in_progress",
      );
    }
    const action: TodoAction = this.items.length === 0 ? "created" : "updated";
    this.items = normalized;
    return { action, todos: this.current() };
  }

  onSessionActivated(entries: readonly unknown[]): TodoItem[] {
    const restored = entries
      .filter(isJsonObject)
      .filter(
        (entry) =>
          entry.type === "custom" && entry.customType === TODO_ENTRY_TYPE,
      )
      .map((entry) => entry.data)
      .map(parseTodoList)
      .filter((items): items is TodoItem[] => items !== undefined)
      .at(-1);
    this.items = restored ? structuredClone(restored) : [];
    return this.current();
  }
}

function parseTodoList(value: unknown): TodoItem[] | undefined {
  if (!Array.isArray(value)) return undefined;
  const items: TodoItem[] = [];
  for (const raw of value) {
    if (!isJsonObject(raw) || typeof raw.content !== "string") return undefined;
    const content = raw.content.trim();
    const status = raw.status;
    if (
      !content ||
      (status !== "pending" && status !== "in_progress" && status !== "completed")
    ) {
      return undefined;
    }
    items.push({ content, status });
  }
  if (items.filter((item) => item.status === "in_progress").length > 1) {
    return undefined;
  }
  return items;
}
