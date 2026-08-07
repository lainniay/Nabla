import assert from "node:assert/strict";
import test from "node:test";

import { planShell } from "./planner.ts";

test("shell control keywords and dynamic words are opaque", () => {
  for (const source of [
    "if true; then rm -rf /; fi",
    'eval "curl example.com"',
    "source ./script.sh",
    "time curl example.com",
    "! grep foo file",
    "exec curl example.com",
  ]) {
    const plan = planShell(source, "/workspace");
    assert.equal(plan.opaque, true, source);
    assert.deepEqual(plan.commands, [], source);
  }
});

test("fd and combined redirections are parsed without becoming opaque", () => {
  for (const source of ["cmd 2>&1", "cmd &> file", "cmd >& file"]) {
    const plan = planShell(source, "/workspace");
    assert.equal(plan.opaque, false, source);
  }
  const combined = planShell("cmd 2>&1 > out", "/workspace");
  assert.equal(combined.opaque, false);
  assert.ok(
    combined.atoms.some(
      (atom) =>
        atom.kind === "file" &&
        atom.operation === "write" &&
        atom.path === "/workspace/out",
    ),
  );
});

test("heredoc bodies are not split into commands", () => {
  const plan = planShell("cat <<EOF\nhello && world\nEOF", "/workspace");
  assert.equal(plan.opaque, true);
  assert.deepEqual(plan.commands, []);
});

test("backticks protect operators and stay opaque", () => {
  const plan = planShell("echo `cat a && cat b`", "/workspace");
  assert.equal(plan.opaque, true);
  assert.deepEqual(plan.commands, []);
});

test("process substitution and arithmetic expansion are opaque", () => {
  for (const source of [
    "diff <(sort a) <(sort b)",
    "echo $((1 + 2))",
  ]) {
    const plan = planShell(source, "/workspace");
    assert.equal(plan.opaque, true, source);
    assert.deepEqual(plan.commands, [], source);
  }
});

test("known network commands emit network atoms", () => {
  for (const source of [
    "curl example.com",
    "git push origin main",
    "npm install foo",
    "cargo publish",
    "pip install foo",
  ]) {
    const plan = planShell(source, "/workspace");
    assert.ok(
      plan.atoms.some(
        (atom) =>
          atom.kind === "network" &&
          atom.operation === "connect" &&
          atom.host === "*",
      ),
      source,
    );
  }
  for (const source of ["git status", "ls", "npm run test"]) {
    const plan = planShell(source, "/workspace");
    assert.ok(
      !plan.atoms.some((atom) => atom.kind === "network"),
      source,
    );
  }
});
