import type {
  ShellCommand,
  ShellConnector,
  ShellGroup,
  ShellRedirection,
  ShellScript,
} from "./ast.ts";

interface Segment {
  text: string;
  connector?: ShellConnector;
}

export function parseShell(source: string): ShellScript {
  const trimmed = source.trim();
  if (!trimmed) return { nodes: [], connectors: [], source, opaqueReason: "empty script" };
  try {
    const segments = splitTopLevel(trimmed);
    const nodes = segments.map(({ text }) => parseNode(text));
    return {
      nodes,
      connectors: segments
        .slice(0, -1)
        .map((segment) => segment.connector ?? "sequence"),
      source,
    };
  } catch (error) {
    return {
      nodes: [],
      connectors: [],
      source,
      opaqueReason: error instanceof Error ? error.message : "unparseable shell syntax",
    };
  }
}

function splitTopLevel(source: string): Segment[] {
  const result: Segment[] = [];
  let start = 0;
  let quote: "'" | "\"" | undefined;
  let backtick = false;
  let depth = 0;
  for (let index = 0; index < source.length; index += 1) {
    const character = source[index]!;
    if (character === "\\" && quote !== "'") {
      index += 1;
      continue;
    }
    if (character === "'" || character === "\"") {
      if (quote === character) quote = undefined;
      else if (!quote) quote = character;
      continue;
    }
    if (character === "`" && !quote) {
      backtick = !backtick;
      continue;
    }
    if (quote || backtick) continue;
    if (character === "(") {
      depth += 1;
      continue;
    }
    if (character === ")") {
      depth -= 1;
      if (depth < 0) throw new Error("unmatched closing parenthesis");
      continue;
    }
    if (depth > 0) continue;
    const heredocEnd = readHeredocEnd(source, index);
    if (heredocEnd !== undefined) {
      index = heredocEnd;
      continue;
    }
    const operator = readOperator(source, index);
    if (!operator) continue;
    const text = source.slice(start, index).trim();
    if (!text) throw new Error("missing command near shell operator");
    result.push({ text, connector: operator.connector });
    index += operator.length - 1;
    start = index + 1;
  }
  if (quote) throw new Error("unterminated quote");
  if (depth !== 0) throw new Error("unmatched parenthesis");
  const tail = source.slice(start).trim();
  if (!tail) throw new Error("missing trailing command");
  result.push({ text: tail });
  return result;
}

function readOperator(
  source: string,
  index: number,
): { connector: ShellConnector; length: number } | undefined {
  const pair = source.slice(index, index + 2);
  if (pair === "&&") return { connector: "and", length: 2 };
  if (pair === "||") return { connector: "or", length: 2 };
  if (pair === "|&") return { connector: "pipe_both", length: 2 };
  const character = source[index];
  if (character === "|") return { connector: "pipe", length: 1 };
  if (character === ";") return { connector: "sequence", length: 1 };
  if (character === "\n") return { connector: "sequence", length: 1 };
  if (character === "&" && !isRedirectionAmpersand(source, index)) {
    return { connector: "background", length: 1 };
  }
  return undefined;
}

function isRedirectionAmpersand(source: string, index: number): boolean {
  if (source[index + 1] === ">") return true;
  let previous = index - 1;
  while (previous >= 0 && /\s/u.test(source[previous]!)) previous -= 1;
  if (previous < 0) return false;
  return (
    source[previous] === ">" ||
    source[previous] === "<" ||
    /[0-9]/u.test(source[previous]!)
  );
}

function readHeredocEnd(source: string, index: number): number | undefined {
  if (source[index] !== "<" || source[index + 1] !== "<") return undefined;
  if (source[index + 2] === "<") return undefined;
  let cursor = index + 2;
  while (/\s/u.test(source[cursor] ?? "")) cursor += 1;
  if (source[cursor] === "-") cursor += 1;
  const delimiterStart = cursor;
  while (
    cursor < source.length &&
    !/\s/u.test(source[cursor]!) &&
    !";|&<>".includes(source[cursor]!)
  ) {
    cursor += 1;
  }
  const delimiter = source.slice(delimiterStart, cursor).replace(/^['"]|['"]$/gu, "");
  if (!delimiter) return undefined;
  let lineStart = cursor;
  while (lineStart < source.length) {
    const lineEnd = source.indexOf("\n", lineStart);
    const end = lineEnd === -1 ? source.length : lineEnd;
    if (source.slice(lineStart, end).trim() === delimiter) {
      return end;
    }
    if (lineEnd === -1) return source.length;
    lineStart = lineEnd + 1;
  }
  return source.length;
}

function parseNode(source: string): ShellCommand | ShellGroup {
  const trimmed = source.trim();
  if (trimmed.startsWith("(") && matchingOuterParentheses(trimmed)) {
    return {
      type: "group",
      script: parseShell(trimmed.slice(1, -1)),
      source,
    };
  }
  return parseCommand(source);
}

function matchingOuterParentheses(source: string): boolean {
  let depth = 0;
  let quote: "'" | "\"" | undefined;
  let backtick = false;
  for (let index = 0; index < source.length; index += 1) {
    const character = source[index]!;
    if (character === "\\" && quote !== "'") {
      index += 1;
      continue;
    }
    if (character === "'" || character === "\"") {
      if (quote === character) quote = undefined;
      else if (!quote) quote = character;
      continue;
    }
    if (character === "`" && !quote) {
      backtick = !backtick;
      continue;
    }
    if (quote || backtick) continue;
    if (character === "(") depth += 1;
    if (character === ")") {
      depth -= 1;
      if (depth === 0 && index !== source.length - 1) return false;
    }
  }
  return depth === 0;
}

function parseCommand(source: string): ShellCommand {
  const { words, substitutions, redirections, opaqueReason } = lexWords(source);
  const assignments: Record<string, string> = {};
  while (words.length > 0 && /^[A-Za-z_][A-Za-z0-9_]*=/u.test(words[0]!)) {
    const assignment = words.shift()!;
    const equals = assignment.indexOf("=");
    assignments[assignment.slice(0, equals)] = assignment.slice(equals + 1);
  }
  if (words.length === 0 && !opaqueReason) {
    throw new Error("shell fragment has no executable");
  }
  return {
    type: "command",
    argv: words,
    assignments,
    redirections,
    substitutions,
    source,
    ...(opaqueReason ? { opaqueReason } : {}),
  };
}

function lexWords(source: string): {
  words: string[];
  substitutions: ShellScript[];
  redirections: ShellRedirection[];
  opaqueReason?: string;
} {
  const words: string[] = [];
  const substitutions: ShellScript[] = [];
  const redirections: ShellRedirection[] = [];
  let current = "";
  let started = false;
  let quote: "'" | "\"" | undefined;
  let opaqueReason: string | undefined;
  const push = () => {
    if (started) words.push(current);
    current = "";
    started = false;
  };
  for (let index = 0; index < source.length; index += 1) {
    const character = source[index]!;
    if (character === "\\" && quote !== "'") {
      const next = source[index + 1];
      if (next === undefined) {
        opaqueReason = "trailing escape";
        break;
      }
      current += next;
      started = true;
      index += 1;
      continue;
    }
    if (character === "'" || character === "\"") {
      if (quote === character) quote = undefined;
      else if (!quote) quote = character;
      else current += character;
      started = true;
      continue;
    }
    if (
      quote !== "'" &&
      character === "$" &&
      source[index + 1] === "(" &&
      source[index + 2] === "("
    ) {
      opaqueReason = "arithmetic expansion is opaque";
      current += character;
      started = true;
      continue;
    }
    if (quote !== "'" && character === "$" && source[index + 1] === "(") {
      const end = findClosingParenthesis(source, index + 1);
      if (end < 0) {
        opaqueReason = "unterminated command substitution";
        break;
      }
      const nested = source.slice(index + 2, end);
      substitutions.push(parseShell(nested));
      current += `$(${nested})`;
      started = true;
      index = end;
      continue;
    }
    if (quote !== "'" && character === "$") {
      opaqueReason = "dynamic shell variable expansion";
    }
    if (!quote && character === "`") {
      opaqueReason = "backtick command substitution is opaque";
      current += character;
      started = true;
      continue;
    }
    if (
      !quote &&
      (character === "<" || character === ">") &&
      source[index + 1] === "("
    ) {
      opaqueReason = "process substitution is opaque";
      current += character;
      started = true;
      continue;
    }
    if (!quote && character === "<" && source[index + 1] === "<") {
      opaqueReason = "heredoc or herestring is opaque";
      current += character;
      started = true;
      continue;
    }
    if (!quote && character === "&" && source[index + 1] === ">") {
      push();
      let targetStart = index + 2;
      while (/\s/u.test(source[targetStart] ?? "")) targetStart += 1;
      const target = readRedirectionTarget(source, targetStart);
      if (!target.value) {
        opaqueReason = "redirection target is dynamic or missing";
        break;
      }
      redirections.push({
        operation: "write",
        target: target.value,
      });
      index = target.end;
      continue;
    }
    if (!quote && /\s/u.test(character)) {
      push();
      continue;
    }
    if (!quote && (character === ">" || character === "<" ||
        (/[0-9]/u.test(character) && source[index + 1] === ">"))) {
      push();
      const fd = /[0-9]/u.test(character) ? Number(character) : undefined;
      if (fd !== undefined) index += 1;
      const symbol = source[index]!;
      if (symbol === ">" && source[index + 1] === "&") {
        let targetStart = index + 2;
        while (/\s/u.test(source[targetStart] ?? "")) targetStart += 1;
        const target = readRedirectionTarget(source, targetStart);
        if (!target.value) {
          opaqueReason = "redirection target is dynamic or missing";
          break;
        }
        if (!/^[0-9]+$/u.test(target.value)) {
          redirections.push({
            operation: "write",
            target: target.value,
          });
        }
        index = target.end;
        continue;
      }
      const append = symbol === ">" && source[index + 1] === ">";
      if (append) index += 1;
      while (/\s/u.test(source[index + 1] ?? "")) index += 1;
      const targetStart = index + 1;
      const target = readRedirectionTarget(source, targetStart);
      if (!target.value) {
        opaqueReason = "redirection target is dynamic or missing";
        break;
      }
      redirections.push({
        ...(fd === undefined ? {} : { fd }),
        operation: symbol === "<" ? "read" : append ? "append" : "write",
        target: target.value,
      });
      index = target.end;
      continue;
    }
    if (!quote && ["{", "}", "~"].includes(character)) {
      opaqueReason = "shell expansion is not statically understood";
    }
    current += character;
    started = true;
  }
  push();
  if (quote) opaqueReason = "unterminated quote";
  return { words, substitutions, redirections, ...(opaqueReason ? { opaqueReason } : {}) };
}

function findClosingParenthesis(source: string, opening: number): number {
  let depth = 0;
  let quote: "'" | "\"" | undefined;
  let backtick = false;
  for (let index = opening; index < source.length; index += 1) {
    const character = source[index]!;
    if (character === "\\" && quote !== "'") {
      index += 1;
      continue;
    }
    if (character === "'" || character === "\"") {
      if (quote === character) quote = undefined;
      else if (!quote) quote = character;
      continue;
    }
    if (character === "`" && !quote) {
      backtick = !backtick;
      continue;
    }
    if (quote || backtick) continue;
    if (character === "(") depth += 1;
    if (character === ")" && --depth === 0) return index;
  }
  return -1;
}

function readRedirectionTarget(
  source: string,
  start: number,
): { value: string; end: number } {
  if (start >= source.length) return { value: "", end: start };
  const quote = source[start] === "'" || source[start] === "\"" ? source[start] : undefined;
  let value = "";
  let index = quote ? start + 1 : start;
  for (; index < source.length; index += 1) {
    const character = source[index]!;
    if (quote && character === quote) return { value, end: index };
    if (!quote && (/\s/u.test(character) || ";&|<>".includes(character))) break;
    if (character === "$" || character === "`") return { value: "", end: index };
    value += character;
  }
  return { value, end: index - 1 };
}
