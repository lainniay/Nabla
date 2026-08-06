import { isJsonObject, type JsonObject } from "./validation.ts";
import type { OperationContext } from "../app/operation-scope.ts";

export interface CommandDefinition<
  TRequest extends JsonObject = JsonObject,
  TResponse = unknown,
> {
  readonly type: string;
  readonly lane:
    | string
    | ((request: JsonObject) => string | undefined)
    | undefined;
  decode(value: unknown): TRequest;
  handle(
    context: OperationContext,
    request: TRequest,
  ): Promise<TResponse> | TResponse;
}

export function requestObject(value: unknown): JsonObject {
  if (!isJsonObject(value)) {
    throw new Error("Host request must be a JSON object");
  }
  return value;
}
