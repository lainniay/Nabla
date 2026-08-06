import { isJsonObject, type JsonObject } from "./validation.ts";

export interface CommandDefinition<Input extends JsonObject = JsonObject> {
  name: string;
  lane?: string;
  decode(request: unknown): Input;
}

export function requestObject(value: unknown): JsonObject {
  if (!isJsonObject(value)) {
    throw new Error("Host request must be a JSON object");
  }
  return value;
}
