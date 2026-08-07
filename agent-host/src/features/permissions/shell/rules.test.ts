import assert from "node:assert/strict";
import test from "node:test";

import {
  isDangerousExecCommand,
  isHighRiskCommand,
  isReadOnlyGitCommand,
} from "./rules.ts";

test("high-risk command detection is advisory and identifies UI warnings", () => {
  assert.equal(isHighRiskCommand("rm -rf target"), true);
  assert.equal(isHighRiskCommand("cargo test"), false);
});

test("shell classifier keeps the only read-only git implementation", () => {
  assert.equal(isReadOnlyGitCommand(["log", "--oneline"], "/workspace"), true);
  assert.equal(isReadOnlyGitCommand(["push"], "/workspace"), false);
  assert.equal(
    isDangerousExecCommand("/usr/bin/git", ["push"], "/workspace"),
    true,
  );
});
