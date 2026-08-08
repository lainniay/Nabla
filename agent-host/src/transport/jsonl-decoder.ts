import {
  errorMessage,
  isJsonObject,
  type JsonObject,
} from "../protocol/validation.ts";
import {
  FrameTooLargeError,
  JsonlParseError,
  JsonlRequestError,
} from "./transport-errors.ts";

export const MAX_CONTROL_FRAME_BYTES = 1_048_576;

export class JsonlDecoder {
  private buffered = "";
  private readonly maxFrameBytes: number;

  constructor(maxFrameBytes = MAX_CONTROL_FRAME_BYTES) {
    this.maxFrameBytes = maxFrameBytes;
  }

  push(chunk: string): JsonObject[] {
    this.buffered += chunk;
    const frames: JsonObject[] = [];
    while (true) {
      const newline = this.buffered.indexOf("\n");
      if (newline < 0) break;
      const line = this.buffered.slice(0, newline).replace(/\r$/u, "");
      this.buffered = this.buffered.slice(newline + 1);
      if (line.length === 0) continue;
      if (line.length > this.maxFrameBytes) {
        this.buffered = "";
        throw new FrameTooLargeError(
          `Control frame exceeds ${this.maxFrameBytes} bytes`,
        );
      }
      frames.push(this.parseLine(line));
    }
    if (this.buffered.length > this.maxFrameBytes) {
      this.buffered = "";
      throw new FrameTooLargeError(
        `Control frame exceeds ${this.maxFrameBytes} bytes`,
      );
    }
    return frames;
  }

  flush(): void {
    this.buffered = "";
  }

  private parseLine(line: string): JsonObject {
    let parsed: unknown;
    try {
      parsed = JSON.parse(line);
    } catch (error) {
      throw new JsonlParseError(
        errorMessage(error),
      );
    }
    if (!isJsonObject(parsed)) {
      throw new JsonlRequestError("Host request must be a JSON object");
    }
    return parsed;
  }
}
