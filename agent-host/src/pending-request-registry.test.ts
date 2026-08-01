import assert from "node:assert/strict";
import test from "node:test";

import { PendingRequestRegistry } from "./protocol/pending-request-registry.ts";

test("pending registry consumes and cleans each request exactly once", () => {
  const registry = new PendingRequestRegistry<number>();
  let cleanups = 0;
  registry.register("one", 1, () => {
    cleanups += 1;
  });

  assert.equal(registry.take("one"), 1);
  assert.equal(registry.take("one"), undefined);
  assert.equal(cleanups, 1);
  assert.equal(registry.size, 0);
});

test("pending registry drains a stable snapshot before domain settlement", () => {
  const registry = new PendingRequestRegistry<number>();
  const cleaned: number[] = [];
  registry.register("one", 1, () => cleaned.push(1));
  registry.register("two", 2, () => cleaned.push(2));

  assert.deepEqual(registry.drain(), [1, 2]);
  assert.deepEqual(cleaned, [1, 2]);
  assert.equal(registry.size, 0);
});
