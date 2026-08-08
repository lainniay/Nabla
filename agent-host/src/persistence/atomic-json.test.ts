import assert from "node:assert/strict";
import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";

import {
  writeAtomicFile,
  writeAtomicJson,
  writeAtomicJsonSync,
} from "./atomic-json.ts";

test("atomic JSON writes round-trip with 0600 mode and leave no temp files", async () => {
  const root = mkdtempSync(join(tmpdir(), "nabla-atomic-"));
  try {
    const target = join(root, "state.json");
    await writeAtomicJson(target, { a: 1 });
    assert.deepEqual(JSON.parse(readFileSync(target, "utf8")), { a: 1 });
    assert.equal(statSync(target).mode & 0o777, 0o600);

    writeAtomicJsonSync(target, { b: 2 });
    assert.deepEqual(JSON.parse(readFileSync(target, "utf8")), { b: 2 });
    assert.deepEqual(readdirSync(root), ["state.json"]);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("writeAtomicFile round-trips bytes and the sync variant creates parent directories", async () => {
  const root = mkdtempSync(join(tmpdir(), "nabla-atomic-"));
  try {
    await writeAtomicFile(join(root, "data.bin"), new Uint8Array([1, 2, 3]));
    assert.deepEqual(
      [...readFileSync(join(root, "data.bin"))],
      [1, 2, 3],
    );

    const nested = join(root, "nested", "state.json");
    writeAtomicJsonSync(nested, { ok: true });
    assert.deepEqual(JSON.parse(readFileSync(nested, "utf8")), { ok: true });
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("failed atomic writes remove the temporary file and leave the target untouched", async () => {
  const root = mkdtempSync(join(tmpdir(), "nabla-atomic-"));
  try {
    const blocked = join(root, "blocked");
    mkdirSync(blocked);
    await assert.rejects(() => writeAtomicFile(blocked, "x"));
    assert.throws(() => writeAtomicJsonSync(blocked, { x: 1 }));
    assert.deepEqual(readdirSync(root), ["blocked"]);
    assert.deepEqual(readdirSync(blocked), []);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
