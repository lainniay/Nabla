import type { Socket } from "node:net";

import type { JsonObject } from "../protocol/validation.ts";

export class OutboundWriter {
  private readonly socket: Socket;
  private tail: Promise<void> = Promise.resolve();

  constructor(socket: Socket) {
    this.socket = socket;
  }

  write(message: JsonObject): void {
    this.tail = this.tail
      .then(() => this.deliver(message))
      .catch(() => undefined);
  }

  private deliver(message: JsonObject): Promise<void> {
    if (this.socket.destroyed) return Promise.resolve();
    const payload = `${JSON.stringify(message)}\n`;
    return new Promise<void>((resolve) => {
      const cleanup = () => {
        this.socket.off("drain", onDrain);
        this.socket.off("close", onClose);
        this.socket.off("error", onError);
      };
      const onDrain = () => {
        cleanup();
        resolve();
      };
      const onClose = () => {
        cleanup();
        resolve();
      };
      const onError = () => {
        cleanup();
        resolve();
      };
      this.socket.once("drain", onDrain);
      this.socket.once("close", onClose);
      this.socket.once("error", onError);
      if (this.socket.write(payload)) {
        cleanup();
        resolve();
      }
    });
  }
}
