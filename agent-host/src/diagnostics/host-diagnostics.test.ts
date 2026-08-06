import assert from "node:assert/strict";
import test from "node:test";

import { HostDiagnostics } from "./host-diagnostics.ts";

test("warnings preserve order, deduplicate, and snapshot without leaking context", () => {
  const diagnostics = new HostDiagnostics();
  diagnostics.warn("first");
  diagnostics.warn("first");
  diagnostics.warn("second", { token: "super-secret" });
  assert.deepEqual(diagnostics.snapshot(), ["first", "second"]);
  assert.equal(diagnostics.snapshot().includes("super-secret"), false);
});

test("snapshot returns a copy that cannot mutate the store", () => {
  const diagnostics = new HostDiagnostics();
  diagnostics.warn("w");
  const snapshot = diagnostics.snapshot() as string[];
  snapshot.push("tampered");
  assert.deepEqual(diagnostics.snapshot(), ["w"]);
});
