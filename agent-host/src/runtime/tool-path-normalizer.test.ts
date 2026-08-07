import assert from "node:assert/strict";
import { join } from "node:path";
import test from "node:test";

import {
  normalizePath,
  normalizeToolInputPaths,
} from "./tool-path-normalizer.ts";

test("normalizePath rewrites workspace paths to relative", () => {
  assert.equal(normalizePath("/workspace/src/lib.rs", "/workspace"), "src/lib.rs");
  assert.equal(normalizePath("/workspace", "/workspace"), ".");
  assert.equal(normalizePath("/etc/passwd", "/workspace"), undefined);
  assert.equal(normalizePath("src/lib.rs", "/workspace"), undefined);
});

test("normalizeToolInputPaths handles path and destination", () => {
  const input = {
    path: join("/workspace", "src", "a.ts"),
    destination: join("/workspace", "src", "b.ts"),
    pattern: "*.ts",
  };
  normalizeToolInputPaths(input, "/workspace");
  assert.equal(input.path, "src/a.ts");
  assert.equal(input.destination, "src/b.ts");
  assert.equal(input.pattern, "*.ts");
});
