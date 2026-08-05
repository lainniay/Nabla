import assert from "node:assert/strict";
import test from "node:test";

import { CommandLanes } from "./protocol/command-lanes.ts";

test("command lanes serialize one domain while allowing unrelated queries", async () => {
  const lanes = new CommandLanes();
  const events: string[] = [];
  let release!: () => void;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  const first = lanes.run("session", async () => {
    events.push("session-1-start");
    await gate;
    events.push("session-1-end");
  });
  const second = lanes.run("session", async () => {
    events.push("session-2");
  });
  await Promise.resolve();
  await lanes.run(undefined, async () => {
    events.push("query");
  });
  assert.deepEqual(events, ["session-1-start", "query"]);
  release();
  await Promise.all([first, second]);
  assert.deepEqual(events, [
    "session-1-start",
    "query",
    "session-1-end",
    "session-2",
  ]);
});

test("command lane continues after a failed mutation", async () => {
  const lanes = new CommandLanes();
  const first = lanes.run("mutation", async () => {
    throw new Error("failed");
  });
  const second = lanes.run("mutation", async () => "continued");
  await assert.rejects(first, /failed/u);
  assert.equal(await second, "continued");
});
