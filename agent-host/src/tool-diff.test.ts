import assert from "node:assert/strict";
import test from "node:test";

import { newFileDisplayDiff } from "./tool-diff.ts";

test("new files are represented as numbered additions", () => {
  assert.equal(
    newFileDisplayDiff("first\nsecond\n"),
    "+1 first\n+2 second",
  );
  assert.equal(newFileDisplayDiff("first\r\nsecond"), "+1 first\n+2 second");
});

test("empty new files have no visible diff", () => {
  assert.equal(newFileDisplayDiff(""), undefined);
});
