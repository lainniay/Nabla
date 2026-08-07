import assert from "node:assert/strict";
import test from "node:test";

import { parseSubagentOutput } from "./protocol/subagent-output.ts";

test("strict subagent output rejects malformed and unknown task results", () => {
  assert.throws(
    () => parseSubagentOutput("not json"),
    /valid JSON object/u,
  );
  assert.throws(
    () =>
      parseSubagentOutput(
        JSON.stringify({
          status: "done",
          summary: "looks fine",
          evidence: [],
          changedPaths: [],
          verification: [],
          blockers: [],
        }),
      ),
    /must match a schema in anyOf|must be equal to constant/u,
  );
});

test("strict subagent output accepts fenced task results", () => {
  const parsed = parseSubagentOutput(
    `\`\`\`json\n${JSON.stringify({
      status: "completed",
      summary: "done",
      evidence: ["test passed"],
      changedPaths: ["src/file.ts"],
      verification: [{ command: "npm test", exitCode: 0, output: "ok" }],
      blockers: [],
    })}\n\`\`\``,
  );
  assert.equal(parsed.status, "completed");
});
