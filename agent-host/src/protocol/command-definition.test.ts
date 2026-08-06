import assert from "node:assert/strict";
import test from "node:test";

import { requestObject } from "./command-definition.ts";
import {
  enumField,
  optionalNonNegativeIntegerField,
  optionalStringField,
  stringArrayField,
  stringField,
} from "./validation.ts";

test("requestObject rejects null, arrays, and primitives", () => {
  for (const value of [null, [1], "x", 1, true, undefined]) {
    assert.throws(() => requestObject(value), /must be a JSON object/u);
  }
  assert.deepEqual(requestObject({ a: 1 }), { a: 1 });
});

test("request field decoders enforce required and optional shapes", () => {
  assert.equal(stringField({ name: "x" }, "name"), "x");
  assert.throws(() => stringField({}, "name"), /Missing string field: name/u);
  assert.throws(
    () => stringField({ name: "" }, "name"),
    /Missing string field: name/u,
  );
  assert.throws(
    () => stringField({ name: 1 }, "name"),
    /Missing string field: name/u,
  );

  assert.equal(optionalStringField({ name: "x" }, "name"), "x");
  assert.equal(optionalStringField({}, "name"), undefined);

  assert.equal(optionalNonNegativeIntegerField({ offset: 2 }, "offset"), 2);
  assert.throws(
    () => optionalNonNegativeIntegerField({ offset: -1 }, "offset"),
    /Invalid non-negative integer field: offset/u,
  );
  assert.throws(
    () => optionalNonNegativeIntegerField({ offset: 1.5 }, "offset"),
    /Invalid non-negative integer field: offset/u,
  );

  assert.deepEqual(stringArrayField({}, "items"), []);
  assert.deepEqual(stringArrayField({ items: ["a"] }, "items"), ["a"]);
  assert.throws(
    () => stringArrayField({ items: [1] }, "items"),
    /Invalid string array field: items/u,
  );

  assert.equal(enumField({ mode: "a" }, "mode", ["a", "b"] as const), "a");
  assert.throws(
    () => enumField({ mode: "c" }, "mode", ["a", "b"] as const),
    /Unsupported mode: c/u,
  );
  assert.throws(
    () => enumField({}, "mode", ["a"] as const),
    /Missing string field: mode/u,
  );
});
