import { chmodSync, rmSync } from "node:fs";
import { createServer, type Socket } from "node:net";

import type { OperationContext } from "../app/operation-scope.ts";
import type { JsonObject } from "../protocol/validation.ts";
import { ControlConnection } from "./control-connection.ts";

export interface LegacyRequestHandler {
  handleRequest(
    context: OperationContext,
    request: JsonObject,
  ): Promise<unknown>;
}

export interface ControlServerLifecycle {
  onConnectionReplaced?(connection: ControlConnection): void;
  onConnectionClosed?(connection: ControlConnection): void;
}

export class ControlServer {
  private readonly socketPath: string;
  private readonly handler: LegacyRequestHandler;
  private readonly lifecycle: ControlServerLifecycle;
  private readonly server = createServer((socket) => this.accept(socket));
  private readonly requestConnections = new Map<string, ControlConnection>();
  private connection?: ControlConnection;
  private connectionGeneration = 0;

  constructor(
    socketPath: string,
    handler: LegacyRequestHandler,
    lifecycle: ControlServerLifecycle = {},
  ) {
    this.socketPath = socketPath;
    this.handler = handler;
    this.lifecycle = lifecycle;
  }

  generation(): number {
    return this.connectionGeneration;
  }

  hasConnection(): boolean {
    return this.connection !== undefined && !this.connection.destroyed;
  }

  isCurrent(context: OperationContext): boolean {
    return (
      context.connectionGeneration === this.connectionGeneration &&
      this.connection?.id === context.connectionId &&
      !this.connection.destroyed
    );
  }

  send(message: JsonObject): void {
    this.connection?.send(message);
  }

  respond(id: string | undefined, message: JsonObject): void {
    const target = id ? this.requestConnections.get(id) : this.connection;
    if (id) this.requestConnections.delete(id);
    target?.send(message);
  }

  async listen(): Promise<void> {
    rmSync(this.socketPath, { force: true });
    await new Promise<void>((resolve, reject) => {
      this.server.once("error", reject);
      this.server.listen(this.socketPath, () => {
        this.server.off("error", reject);
        chmodSync(this.socketPath, 0o600);
        resolve();
      });
    });
  }

  async close(): Promise<void> {
    this.connection?.destroy();
    this.requestConnections.clear();
    await new Promise<void>((resolve) => this.server.close(() => resolve()));
    rmSync(this.socketPath, { force: true });
  }

  private accept(socket: Socket): void {
    const generation = ++this.connectionGeneration;
    const previous = this.connection;
    if (previous) {
      this.forgetConnection(previous);
      this.lifecycle.onConnectionReplaced?.(previous);
      previous.destroy();
    }
    const connection = new ControlConnection(socket, generation);
    this.connection = connection;
    connection.onRequest((request) => this.route(connection, request));
    connection.onClose(() => this.connectionClosed(connection));
  }

  private route(
    connection: ControlConnection,
    request: JsonObject,
  ): void {
    const id = typeof request.id === "string" ? request.id : undefined;
    if (id) this.requestConnections.set(id, connection);
    if (connection !== this.connection || connection.destroyed) {
      if (id) this.requestConnections.delete(id);
      return;
    }
    const context: OperationContext = {
      requestId: id,
      connectionId: connection.id,
      connectionGeneration: connection.generation,
      sessionId: undefined,
      sessionGeneration: 0,
      signal: connection.signal,
    };
    void this.handler.handleRequest(context, request).catch((error) => {
      if (id) this.requestConnections.delete(id);
      connection.send({
        type: "host_protocol_error",
        error: error instanceof Error ? error.message : String(error),
      });
    });
  }

  private connectionClosed(connection: ControlConnection): void {
    this.forgetConnection(connection);
    if (this.connection === connection) {
      this.connection = undefined;
      this.connectionGeneration += 1;
      this.lifecycle.onConnectionClosed?.(connection);
    }
  }

  private forgetConnection(connection: ControlConnection): void {
    for (const [id, target] of this.requestConnections) {
      if (target === connection) this.requestConnections.delete(id);
    }
  }
}
