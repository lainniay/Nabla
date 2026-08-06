import assert from "node:assert/strict";
import test from "node:test";

import { JsonlDecoder } from "./jsonl-decoder.ts";
import {
  FrameTooLargeError,
  JsonlParseError,
  JsonlRequestError,
} from "./transport-errors.ts";

test("decodes one frame per chunk", () => {
  const decoder = new JsonlDecoder();
  assert.deepEqual(decoder.push('{"a":1}\n'), [{ a: 1 }]);
});

test("decodes multiple frames per chunk", () => {
  const decoder = new JsonlDecoder();
  assert.deepEqual(decoder.push('{"a":1}\n{"b":2}\n'), [{ a: 1 }, { b: 2 }]);
});

test("decodes frames split across chunks", () => {
  const decoder = new JsonlDecoder();
  assert.deepEqual(decoder.push('{"a":'), []);
  assert.deepEqual(decoder.push('1}\n{"b":2}\n{"c":'), [{ a: 1 }, { b: 2 }]);
  assert.deepEqual(decoder.push("3}\n"), [{ c: 3 }]);
});

test("skips empty lines and strips carriage returns", () => {
  const decoder = new JsonlDecoder();
  assert.deepEqual(decoder.push('\n\r\n{"a":1}\r\n\n'), [{ a: 1 }]);
});

test("rejects invalid JSON with a parse error", () => {
  const decoder = new JsonlDecoder();
  assert.throws(() => decoder.push("not json\n"), JsonlParseError);
});

test("rejects valid JSON that is not an object", () => {
  const decoder = new JsonlDecoder();
  for (const line of ["null\n", "[1,2]\n", '"text"\n', "42\n"]) {
    assert.throws(() => decoder.push(line), JsonlRequestError);
  }
});

test("rejects oversized frames and recovers after the bad frame", () => {
  const decoder = new JsonlDecoder(32);
  assert.throws(
    () => decoder.push(`{"x":"${"a".repeat(64)}"}\n`),
    FrameTooLargeError,
  );
  assert.deepEqual(decoder.push('{"ok":1}\n'), [{ ok: 1 }]);
});

test("rejects oversized incomplete tails", () => {
  const decoder = new JsonlDecoder(16);
  assert.throws(() => decoder.push(`{"x":"${"a".repeat(32)}`), FrameTooLargeError);
});

test("flush discards an incomplete tail frame", () => {
  const decoder = new JsonlDecoder();
  decoder.push('{"a":1}\n{"b"');
  decoder.flush();
  assert.deepEqual(decoder.push('{"ok":1}\n'), [{ ok: 1 }]);
});
