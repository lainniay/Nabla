import { randomUUID } from "node:crypto";
import type { Socket } from "node:net";

import { errorMessage, type JsonObject } from "../protocol/validation.ts";
import { JsonlDecoder, MAX_CONTROL_FRAME_BYTES } from "./jsonl-decoder.ts";
import { OutboundWriter } from "./outbound-writer.ts";
import { FrameTooLargeError } from "./transport-errors.ts";

export class ControlConnection {
  readonly id = randomUUID();
  readonly generation: number;
  readonly signal: AbortSignal;

  private readonly socket: Socket;
  private readonly controller = new AbortController();
  private readonly decoder = new JsonlDecoder(MAX_CONTROL_FRAME_BYTES);
  private readonly writer: OutboundWriter;
  private readonly requestListeners = new Set<
    (request: JsonObject) => void
  >();
  private readonly closeListeners = new Set<() => void>();
  private closed = false;

  constructor(socket: Socket, generation: number) {
    this.socket = socket;
    this.generation = generation;
    this.signal = this.controller.signal;
    this.writer = new OutboundWriter(socket);
    socket.setEncoding("utf8");
    socket.on("data", (chunk: string) => this.onData(chunk));
    socket.on("close", () => this.onClosed());
    socket.on("error", () => {});
  }

  get destroyed(): boolean {
    return this.closed;
  }

  onRequest(listener: (request: JsonObject) => void): void {
    this.requestListeners.add(listener);
  }

  onClose(listener: () => void): void {
    this.closeListeners.add(listener);
  }

  send(message: JsonObject): void {
    this.writer.write(message);
  }

  destroy(): void {
    this.socket.destroy();
  }

  private onData(chunk: string): void {
    let frames: JsonObject[];
    try {
      frames = this.decoder.push(chunk);
    } catch (error) {
      this.send({
        type: "host_protocol_error",
        error: errorMessage(error),
      });
      if (error instanceof FrameTooLargeError) {
        setImmediate(() => this.destroy());
      }
      return;
    }
    for (const request of frames) {
      for (const listener of this.requestListeners) listener(request);
    }
  }

  private onClosed(): void {
    if (this.closed) return;
    this.closed = true;
    this.decoder.flush();
    this.controller.abort();
    for (const listener of this.closeListeners) listener();
  }
}
