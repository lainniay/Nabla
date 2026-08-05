import assert from "node:assert/strict";
import test from "node:test";

import {
  isHighRiskCommand,
} from "./policy/tool-policy.ts";

test("high-risk command detection is advisory and identifies UI warnings", () => {
  assert.equal(isHighRiskCommand("rm -rf target"), true);
  assert.equal(isHighRiskCommand("cargo test"), false);
});
