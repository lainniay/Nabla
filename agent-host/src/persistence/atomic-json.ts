import { open, rename, rm } from "node:fs/promises";
import { dirname } from "node:path";
import { randomUUID } from "node:crypto";
import {
  closeSync,
  fsyncSync,
  mkdirSync,
  openSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs";

/**
 * Replace a JSON sidecar atomically without sharing a temporary name between
 * concurrent writers. The file is fsynced before rename so a successful call
 * means the new document reached the filesystem, not only the Node write
 * buffer.
 */
export async function writeAtomicFile(
  path: string,
  content: string | Uint8Array,
  mode = 0o600,
): Promise<void> {
  const temporary = `${path}.tmp-${process.pid}-${randomUUID()}`;
  const handle = await open(temporary, "wx", mode);
  try {
    await handle.writeFile(content);
    await handle.sync();
  } catch (error) {
    await handle.close().catch(() => undefined);
    await rm(temporary, { force: true }).catch(() => undefined);
    throw error;
  }
  await handle.close();
  try {
    await rename(temporary, path);
    const directory = await open(dirname(path), "r").catch(() => undefined);
    if (directory) {
      await directory.sync().catch(() => undefined);
      await directory.close().catch(() => undefined);
    }
  } catch (error) {
    await rm(temporary, { force: true }).catch(() => undefined);
    throw error;
  }
}

export async function writeAtomicJson(
  path: string,
  value: unknown,
  mode = 0o600,
): Promise<void> {
  await writeAtomicFile(path, `${JSON.stringify(value, null, 2)}\n`, mode);
}

export function writeAtomicJsonSync(
  path: string,
  value: unknown,
  mode = 0o600,
): void {
  mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
  const temporary = `${path}.tmp-${process.pid}-${randomUUID()}`;
  let descriptor: number | undefined;
  try {
    descriptor = openSync(temporary, "wx", mode);
    writeFileSync(descriptor, `${JSON.stringify(value, null, 2)}\n`, "utf8");
    fsyncSync(descriptor);
    closeSync(descriptor);
    descriptor = undefined;
    renameSync(temporary, path);
    const directory = openSync(dirname(path), "r");
    try {
      fsyncSync(directory);
    } finally {
      closeSync(directory);
    }
  } catch (error) {
    if (descriptor !== undefined) closeSync(descriptor);
    rmSync(temporary, { force: true });
    throw error;
  }
}
