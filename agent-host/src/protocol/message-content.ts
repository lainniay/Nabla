import { isJsonObject, stringArray } from "./validation.ts";

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
