import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import type { Socket } from "node:net";
import test from "node:test";

import { OutboundWriter } from "./outbound-writer.ts";

class FakeSocket extends EventEmitter {
  destroyed = false;
  readonly written: string[] = [];
  pendingDrain = false;

  write(chunk: string): boolean {
    this.written.push(chunk);
    return !this.pendingDrain;
  }

  destroy(): void {
    this.destroyed = true;
    this.emit("close");
  }
}

const tick = () => new Promise<void>((resolve) => setImmediate(resolve));

test("writes preserve call order", async () => {
  const socket = new FakeSocket();
  const writer = new OutboundWriter(socket as unknown as Socket);
  writer.write({ a: 1 });
  writer.write({ b: 2 });
  await tick();
  await tick();
  assert.deepEqual(socket.written, ['{"a":1}\n', '{"b":2}\n']);
});

test("backpressure pauses writes until drain", async () => {
  const socket = new FakeSocket();
  socket.pendingDrain = true;
  const writer = new OutboundWriter(socket as unknown as Socket);
  writer.write({ a: 1 });
  writer.write({ b: 2 });
  await tick();
  assert.deepEqual(socket.written, ['{"a":1}\n']);
  socket.pendingDrain = false;
  socket.emit("drain");
  await tick();
  await tick();
  assert.deepEqual(socket.written, ['{"a":1}\n', '{"b":2}\n']);
});

test("destroyed sockets drop pending writes without stalling", async () => {
  const socket = new FakeSocket();
  const writer = new OutboundWriter(socket as unknown as Socket);
  writer.write({ a: 1 });
  await tick();
  await tick();
  socket.destroy();
  writer.write({ b: 2 });
  await tick();
  await tick();
  assert.deepEqual(socket.written, ['{"a":1}\n']);
});
