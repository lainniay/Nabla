import { isJsonObject, stringArray } from "./validation.ts";

export const FILE_REFERENCE_ENVELOPE_PREFIX = "NABLA_FILE_REFERENCES_V1\n";

export interface FileReferenceEnvelope {
  version: 1;
  message: string;
  references: Array<{
    path: string;
    mode: "snapshot" | "path" | "image";
    size: number;
    reason?: string;
    content?: string;
  }>;
}

export function parseFileReferenceEnvelope(
  text: string,
): FileReferenceEnvelope | undefined {
  if (!text.startsWith(FILE_REFERENCE_ENVELOPE_PREFIX)) return undefined;
  try {
    const payload = text
      .slice(FILE_REFERENCE_ENVELOPE_PREFIX.length)
      .split("\n", 1)[0];
    const value: unknown = JSON.parse(payload);
    if (
      !isJsonObject(value) ||
      value.version !== 1 ||
      typeof value.message !== "string" ||
      !Array.isArray(value.references)
    ) {
      return undefined;
    }
    return value as unknown as FileReferenceEnvelope;
  } catch {
    return undefined;
  }
}

export function displayMessageText(text: string): string {
  return parseFileReferenceEnvelope(text)?.message ?? text;
}

export interface CompactionFileDetails {
  readFiles: string[];
  modifiedFiles: string[];
  fileCount: number;
}

export interface MessageContentTextOptions {
  imageMarker?: string;
  includeThinking?: boolean;
}

export function compactionFileDetails(value: unknown): CompactionFileDetails {
  const details = isJsonObject(value) ? value : {};
  const readFiles = stringArray(details.readFiles);
  const modifiedFiles = stringArray(details.modifiedFiles);
  return {
    readFiles,
    modifiedFiles,
    fileCount: new Set([...readFiles, ...modifiedFiles]).size,
  };
}

export function messageContentText(
  content: unknown,
  options: MessageContentTextOptions = {},
): string {
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return "";
  return content
    .map((part) => {
      if (!isJsonObject(part)) return "";
      if (part.type === "text") {
        return typeof part.text === "string" ? part.text : "";
      }
      if (options.includeThinking && part.type === "thinking") {
        if (typeof part.thinking === "string") return part.thinking;
        return typeof part.text === "string" ? part.text : "";
      }
      if (part.type === "image") return options.imageMarker ?? "";
      return "";
    })
    .filter(Boolean)
    .join("\n");
}
