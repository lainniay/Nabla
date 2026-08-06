import assert from "node:assert/strict";
import test from "node:test";

import type { OperationContext } from "../app/operation-scope.ts";
import type { LegacyHostOperations } from "../legacy-host-operations.ts";
import type { CommandDefinition } from "./command-definition.ts";
import { CommandRouter } from "./command-router.ts";
import { createAgentCommands } from "./commands/agent-commands.ts";
import { createAuthCommands } from "./commands/auth-commands.ts";
import { createBootstrapCommands } from "./commands/bootstrap-commands.ts";
import { createConfigurationCommands } from "./commands/configuration-commands.ts";
import { createInteractionCommands } from "./commands/interaction-commands.ts";
import { createModelCommands } from "./commands/model-commands.ts";
import { createPermissionCommands } from "./commands/permission-commands.ts";
import { createPlanCommands } from "./commands/plan-commands.ts";
import { createSessionCommands } from "./commands/session-commands.ts";
import { requestObject } from "./command-definition.ts";

const HOST_COMMANDS = [
  "agents_reload",
  "agents_state",
  "approval_rule_revoke",
  "approval_rules",
  "approval_rules_clear",
  "approval_reply",
  "auth_cancel",
  "auth_list",
  "auth_login",
  "auth_logout",
  "auth_reply",
  "bootstrap_state",
  "context_state",
  "get_plan_state",
  "model_list",
  "model_set",
  "plan_execute",
  "queue_clear",
  "question_reply",
  "resource_reload",
  "resource_state",
  "session_browser_close",
  "session_browser_open",
  "session_browser_query",
  "session_new",
  "session_resume",
  "set_plan_mode",
  "subagent_cancel",
  "subagent_integrate",
  "subagent_start",
  "thinking_set",
  "tree_abort",
  "tree_copy",
  "tree_label",
  "tree_navigate",
  "tree_state",
  "workspace_trust",
].sort();

const EXPECTED_LANES: Record<string, string | undefined> = {
  agents_reload: "configuration",
  agents_state: undefined,
  approval_rule_revoke: "configuration",
  approval_rules: "configuration",
  approval_rules_clear: "configuration",
  approval_reply: undefined,
  auth_cancel: "auth",
  auth_list: undefined,
  auth_login: "auth",
  auth_logout: "auth",
  auth_reply: "auth",
  bootstrap_state: undefined,
  context_state: undefined,
  get_plan_state: undefined,
  model_list: undefined,
  model_set: "session",
  plan_execute: "session",
  queue_clear: "session",
  question_reply: undefined,
  resource_reload: "configuration",
  resource_state: undefined,
  session_browser_close: "session-browser",
  session_browser_open: "session-browser",
  session_browser_query: "session-browser",
  session_new: "session",
  session_resume: "session",
  set_plan_mode: "session",
  subagent_cancel: "subagents",
  subagent_integrate: "integration:a1",
  subagent_start: "subagents",
  thinking_set: "session",
  tree_abort: "session",
  tree_copy: undefined,
  tree_label: "session",
  tree_navigate: "session",
  tree_state: undefined,
  workspace_trust: "configuration",
};

function stubOps(overrides: Partial<LegacyHostOperations> = {}): LegacyHostOperations {
  const base = {} as LegacyHostOperations;
  Object.assign(base, overrides);
  return new Proxy(base, {
    get(target, prop) {
      if (prop in target) return target[prop as keyof LegacyHostOperations];
      return async () => undefined;
    },
  });
}

function allCommands(): CommandDefinition[] {
  const ops = stubOps();
  return [
    ...createAuthCommands(ops),
    ...createBootstrapCommands(ops),
    ...createConfigurationCommands(ops),
    ...createInteractionCommands(ops),
    ...createModelCommands(ops),
    ...createPermissionCommands(ops),
    ...createPlanCommands(ops),
    ...createAgentCommands(ops),
    ...createSessionCommands(ops),
  ];
}

const context: OperationContext = {
  connectionId: "c1",
  connectionGeneration: 1,
  sessionGeneration: 0,
  signal: new AbortController().signal,
};

test("every baseline command is registered exactly once", () => {
  const router = new CommandRouter(allCommands());
  assert.deepEqual(router.commandTypes(), HOST_COMMANDS);
});

test("command lanes match the baseline mapping", () => {
  const definitions = allCommands();
  const lanes = new Map(
    definitions.map((definition) => {
      const lane =
        typeof definition.lane === "function"
          ? definition.lane({ agentId: "a1" })
          : definition.lane;
      return [definition.type, lane];
    }),
  );
  assert.deepEqual(Object.fromEntries(lanes), EXPECTED_LANES);
});

test("unknown commands return a compatible failure envelope", async () => {
  const router = new CommandRouter([]);
  const result = await router.route(context, { id: "r1", type: "nope" });
  assert.deepEqual(result, {
    id: "r1",
    envelope: {
      id: "r1",
      type: "response",
      command: "nope",
      success: false,
      error: "Unknown host command",
    },
  });
});

test("decode failures do not call the handler", async () => {
  let handled = false;
  const router = new CommandRouter([{
    type: "test",
    lane: undefined,
    decode: () => {
      throw new Error("bad field");
    },
    handle: async () => {
      handled = true;
      return { ok: true };
    },
  }]);
  const result = await router.route(context, { id: "r1", type: "test" });
  assert.equal(handled, false);
  assert.equal(result?.envelope.success, false);
  assert.equal(result?.envelope.error, "bad field");
});

test("handler errors become compatible failure responses", async () => {
  const router = new CommandRouter([{
    type: "test",
    lane: undefined,
    decode: requestObject,
    handle: async () => {
      throw new Error("handler exploded");
    },
  }]);
  const result = await router.route(context, { id: "r1", type: "test" });
  assert.equal(result?.envelope.success, false);
  assert.equal(result?.envelope.error, "handler exploded");
});

test("handler data is placed into the success response unchanged", async () => {
  const router = new CommandRouter([{
    type: "test",
    lane: undefined,
    decode: requestObject,
    handle: async () => ({ value: 42 }),
  }]);
  const result = await router.route(context, { id: "r1", type: "test" });
  assert.equal(result?.envelope.success, true);
  assert.deepEqual(result?.envelope.data, { value: 42 });
});

test("cancelled requests produce no response and skip the handler", async () => {
  let handled = false;
  const router = new CommandRouter(
    [{
      type: "test",
      lane: undefined,
      decode: requestObject,
      handle: async () => {
        handled = true;
        return undefined;
      },
    }],
    () => false,
  );
  const result = await router.route(context, { id: "r1", type: "test" });
  assert.equal(result, undefined);
  assert.equal(handled, false);
});

test("commands in the same lane serialize and different lanes run concurrently", async () => {
  const events: string[] = [];
  let release!: () => void;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  const gated: CommandDefinition = {
    type: "gated",
    lane: "session",
    decode: requestObject,
    handle: async () => {
      events.push("gated-start");
      await gate;
      events.push("gated-end");
      return undefined;
    },
  };
  const queued: CommandDefinition = {
    type: "queued",
    lane: "session",
    decode: requestObject,
    handle: async () => {
      events.push("queued");
      return undefined;
    },
  };
  const parallel: CommandDefinition = {
    type: "parallel",
    lane: "other",
    decode: requestObject,
    handle: async () => {
      events.push("parallel");
      return undefined;
    },
  };
  const router = new CommandRouter([gated, queued, parallel]);
  const first = router.route(context, { id: "1", type: "gated" });
  const second = router.route(context, { id: "2", type: "queued" });
  const third = router.route(context, { id: "3", type: "parallel" });
  await Promise.resolve();
  assert.deepEqual(events, ["gated-start", "parallel"]);
  release();
  await Promise.all([first, second, third]);
  assert.deepEqual(events, [
    "gated-start",
    "parallel",
    "gated-end",
    "queued",
  ]);
});

test("duplicate command registrations are rejected", () => {
  const definition: CommandDefinition = {
    type: "dup",
    lane: undefined,
    decode: requestObject,
    handle: async () => undefined,
  };
  assert.throws(
    () => new CommandRouter([definition, definition]),
    /Duplicate command: dup/u,
  );
});

test("empty type falls back to the unknown command response", async () => {
  const router = new CommandRouter([]);
  const result = await router.route(context, { id: "r1" });
  assert.equal(result?.envelope.command, "unknown");
  assert.equal(result?.envelope.success, false);
});
