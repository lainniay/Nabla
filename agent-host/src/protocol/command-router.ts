import type { OperationContext } from "../app/operation-scope.ts";
import { CommandLanes } from "./command-lanes.ts";
import type { CommandDefinition } from "./command-definition.ts";
import type { JsonObject } from "./validation.ts";

export interface RouteResult {
  id: string | undefined;
  envelope: JsonObject;
}

export class CommandRouter {
  private readonly definitions = new Map<
    string,
    CommandDefinition<JsonObject>
  >();
  private readonly lanes = new CommandLanes();
  private readonly shouldRun: (context: OperationContext) => boolean;

  constructor(
    definitions: readonly CommandDefinition<JsonObject>[],
    shouldRun: (context: OperationContext) => boolean = () => true,
  ) {
    this.shouldRun = shouldRun;
    for (const definition of definitions) {
      if (this.definitions.has(definition.type)) {
        throw new Error(`Duplicate command: ${definition.type}`);
      }
      this.definitions.set(definition.type, definition);
    }
  }

  commandTypes(): readonly string[] {
    return [...this.definitions.keys()].sort();
  }

  async route(
    context: OperationContext,
    request: JsonObject,
  ): Promise<RouteResult | undefined> {
    const id = typeof request.id === "string" ? request.id : undefined;
    const rawType = typeof request.type === "string" ? request.type : "";
    const definition = this.definitions.get(rawType);
    const lane = definition
      ? typeof definition.lane === "function"
        ? definition.lane(request)
        : definition.lane
      : undefined;
    const envelope = await this.lanes.run(lane, async () => {
      if (!this.shouldRun(context)) return undefined;
      if (!definition) {
        return {
          id,
          type: "response",
          command: rawType || "unknown",
          success: false,
          error: "Unknown host command",
        };
      }
      let decoded: JsonObject;
      try {
        decoded = definition.decode(request);
      } catch (error) {
        return {
          id,
          type: "response",
          command: rawType,
          success: false,
          error: error instanceof Error ? error.message : String(error),
        };
      }
      try {
        const data = await definition.handle(context, decoded);
        return {
          id,
          type: "response",
          command: rawType,
          success: true,
          ...(data === undefined ? {} : { data }),
        };
      } catch (error) {
        return {
          id,
          type: "response",
          command: rawType,
          success: false,
          error: error instanceof Error ? error.message : String(error),
        };
      }
    });
    if (envelope === undefined) return undefined;
    return { id, envelope };
  }
}
