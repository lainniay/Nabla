export function newFileDisplayDiff(content: string): string | undefined {
  if (content.length === 0) return undefined;

  const lines = content.split(/\r?\n/);
  if (lines.at(-1) === "") lines.pop();
  if (lines.length === 0) return undefined;

  return lines.map((line, index) => `+${index + 1} ${line}`).join("\n");
}
