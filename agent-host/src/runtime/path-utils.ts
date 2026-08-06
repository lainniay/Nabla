import { homedir } from "node:os";
import { resolve } from "node:path";

export function expandHomePath(value: string): string {
  if (value === "~") return homedir();
  if (value.startsWith("~/")) return resolve(homedir(), value.slice(2));
  return value;
}
