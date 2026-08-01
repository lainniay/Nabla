import assert from "node:assert/strict";
import test from "node:test";

import { parseSubagentOutput } from "./protocol/subagent-output.ts";

test("strict subagent output rejects malformed and unknown task results", () => {
  assert.throws(
    () => parseSubagentOutput("not json", "task"),
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
        "task",
      ),
    /status/u,
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
    "task",
  );
  assert.equal(parsed.status, "completed");
});

test("strict goal and review outputs require their complete shapes", () => {
  assert.throws(
    () => parseSubagentOutput('{"summary":"empty"}', "goal_spec"),
    /acceptanceCriteria/u,
  );
  assert.throws(
    () =>
      parseSubagentOutput(
        '{"verdict":"pass","summary":"ok","findings":[{"severity":"urgent"}]}',
        "review",
      ),
    /severity/u,
  );
});
