import assert from "node:assert/strict";
import { existsSync, mkdtempSync, rmSync, statSync } from "node:fs";
import { createConnection, type Socket } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import type { CommandDefinition } from "../protocol/command-definition.ts";
import { requestObject } from "../protocol/command-definition.ts";
import { CommandRouter } from "../protocol/command-router.ts";
import type { JsonObject } from "../protocol/validation.ts";
import { ControlServer } from "./control-server.ts";
import { MAX_CONTROL_FRAME_BYTES } from "./jsonl-decoder.ts";

const delay = (ms: number) =>
  new Promise<void>((resolve) => setTimeout(resolve, ms));

class TestClient {
  private buffer = "";
  private readonly socket: Socket;
  readonly messages: JsonObject[] = [];
  private readonly closedPromise: Promise<void>;

  constructor(socket: Socket) {
    this.socket = socket;
    this.socket.setEncoding("utf8");
    this.socket.on("data", (chunk: string) => {
      this.buffer += chunk;
      while (true) {
        const newline = this.buffer.indexOf("\n");
        if (newline < 0) break;
        const line = this.buffer.slice(0, newline);
        this.buffer = this.buffer.slice(newline + 1);
        if (!line) continue;
        this.messages.push(JSON.parse(line) as JsonObject);
      }
    });
    this.closedPromise = new Promise((resolve) => socket.once("close", resolve));
  }

  write(data: string): void {
    this.socket.write(data);
  }

  close(): Promise<void> {
    this.socket.destroy();
    return this.closedPromise;
  }

  get closed(): Promise<void> {
    return this.closedPromise;
  }

  async waitFor(
    predicate: (message: JsonObject) => boolean,
    timeoutMs = 1_000,
  ): Promise<JsonObject> {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      const match = this.messages.find(predicate);
      if (match) return match;
      await new Promise((resolve) => setTimeout(resolve, 10));
    }
    throw new Error("timed out waiting for message");
  }
}

function connect(socketPath: string): Promise<TestClient> {
  return new Promise((resolve, reject) => {
    const socket = createConnection(socketPath);
    socket.once("connect", () => resolve(new TestClient(socket)));
    socket.once("error", reject);
  });
}

function command(
  type: string,
  handle: CommandDefinition["handle"],
  lane?: CommandDefinition["lane"],
): CommandDefinition {
  return { type, lane, decode: requestObject, handle };
}

async function withServer(
  definitions: CommandDefinition[],
  run: (server: ControlServer, socketPath: string) => Promise<void>,
): Promise<void> {
  const root = mkdtempSync(join(tmpdir(), "nabla-control-server-"));
  const socketPath = join(root, "control.sock");
  const server = new ControlServer(socketPath, new CommandRouter(definitions));
  await server.listen();
  try {
    await run(server, socketPath);
  } finally {
    await server.close();
    rmSync(root, { recursive: true, force: true });
  }
}

test("multiple frames per chunk are handled in order", async () => {
  await withServer(
    [
      command("a", async () => ({ ok: true })),
      command("b", async () => ({ ok: true })),
    ],
    async (_server, socketPath) => {
      const client = await connect(socketPath);
      client.write('{"id":"1","type":"a"}\n{"id":"2","type":"b"}\n');
      assert.equal((await client.waitFor((m) => m.id === "1")).success, true);
      assert.equal((await client.waitFor((m) => m.id === "2")).success, true);
      await client.close();
    },
  );
});

test("a frame split across chunks is reassembled", async () => {
  await withServer(
    [command("c", async () => ({ ok: true }))],
    async (_server, socketPath) => {
      const client = await connect(socketPath);
      client.write('{"id":"3","ty');
      await delay(20);
      client.write('pe":"c"}\n');
      const message = await client.waitFor((m) => m.id === "3");
      assert.equal(message.command, "c");
      await client.close();
    },
  );
});

test("empty lines are ignored", async () => {
  await withServer(
    [command("d", async () => ({ ok: true }))],
    async (_server, socketPath) => {
      const client = await connect(socketPath);
      client.write('\n\n{"id":"4","type":"d"}\n');
      assert.equal((await client.waitFor((m) => m.id === "4")).command, "d");
      assert.equal(
        client.messages.filter((message) => message.type !== "response").length,
        0,
      );
      await client.close();
    },
  );
});

test("invalid JSON and non-object JSON return protocol errors", async () => {
  await withServer([], async (_server, socketPath) => {
    const client = await connect(socketPath);
    client.write("not json\n");
    const parseError = await client.waitFor(
      (m) => m.type === "host_protocol_error",
    );
    assert.ok(String(parseError.error).length > 0);
    client.write("null\n");
    await delay(20);
    client.write("[1,2]\n");
    await delay(20);
    client.write('"text"\n');
    await delay(20);
    const deadline = Date.now() + 1_000;
    while (
      client.messages.filter((m) => m.type === "host_protocol_error").length <
      4
    ) {
      if (Date.now() > deadline) throw new Error("timed out waiting for errors");
      await delay(10);
    }
    await client.close();
  });
});

test("oversized frames close the connection", async () => {
  await withServer([], async (_server, socketPath) => {
    const client = await connect(socketPath);
    client.write(`{"x":"${"a".repeat(MAX_CONTROL_FRAME_BYTES + 16)}"}\n`);
    const error = await client.waitFor(
      (m) => m.type === "host_protocol_error",
    );
    assert.match(String(error.error), /exceeds/u);
    await client.closed;
  });
});

test("responses stay on the originating connection and stale generations are dropped", async () => {
  let release!: () => void;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  await withServer(
    [
      command("slow", async () => {
        await gate;
        return { ok: true };
      }),
      command("fast", async () => ({ ok: true })),
    ],
    async (_server, socketPath) => {
      const first = await connect(socketPath);
      first.write('{"id":"slow-1","type":"slow"}\n');
      await delay(20);
      const second = await connect(socketPath);
      second.write('{"id":"fresh-1","type":"fast"}\n');
      assert.equal(
        (await second.waitFor((m) => m.id === "fresh-1")).success,
        true,
      );
      release();
      await delay(20);
      assert.equal(
        first.messages.some((message) => message.id === "slow-1"),
        false,
      );
      assert.equal(
        second.messages.some((message) => message.id === "slow-1"),
        false,
      );
      await first.close();
      await second.close();
    },
  );
});

test("connection close cleans request state and the server stays usable", async () => {
  let release!: () => void;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  await withServer(
    [
      command("slow", async () => {
        await gate;
        return { ok: true };
      }),
      command("fast", async () => ({ ok: true })),
    ],
    async (_server, socketPath) => {
      const first = await connect(socketPath);
      first.write('{"id":"gone-1","type":"slow"}\n');
      await delay(20);
      await first.close();
      release();
      await delay(20);
      const second = await connect(socketPath);
      second.write('{"id":"after-1","type":"fast"}\n');
      assert.equal(
        (await second.waitFor((m) => m.id === "after-1")).success,
        true,
      );
      await second.close();
    },
  );
});

test("socket file is created with 0600 and removed on close", async () => {
  const root = mkdtempSync(join(tmpdir(), "nabla-control-server-"));
  const socketPath = join(root, "control.sock");
  const server = new ControlServer(socketPath, new CommandRouter([]));
  await server.listen();
  assert.equal(statSync(socketPath).mode & 0o777, 0o600);
  await server.close();
  assert.equal(existsSync(socketPath), false);
  await server.close();
  rmSync(root, { recursive: true, force: true });
});
