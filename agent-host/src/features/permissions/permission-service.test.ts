import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import type { ToolCallEvent } from "@earendil-works/pi-coding-agent";

import { InteractionBroker } from "../interactions/interaction-broker.ts";
import type { AgentProfile } from "../../harness.ts";
import type { JsonObject } from "../../protocol/validation.ts";
import { PermissionService } from "./permission-service.ts";

const workerProfile: AgentProfile = {
  description: "Worker",
  source: "builtin",
  skills: [],
  tools: ["read", "edit"],
  permission: {},
  maxParallel: 1,
  maxTurns: 10,
  isolation: { mode: "auto", integration: "auto" },
  disabled: false,
  instructions: [],
};

function event(toolCallId: string, toolName: string, input: unknown): ToolCallEvent {
  return { toolCallId, toolName, input } as ToolCallEvent;
}

function service(options: {
  planMode?: boolean;
  connected?: boolean;
  cwd?: string;
  profile?: AgentProfile;
} = {}) {
  const events: JsonObject[] = [];
  const interactions = new InteractionBroker();
  const permissions = new PermissionService(
    interactions,
    (event) => events.push(event),
    { current: () => options.planMode ?? false },
    () => options.connected ?? true,
    {
      sessionId: () => "session-1",
      cwd: () => options.cwd ?? "/workspace",
    },
  );
  return { events, interactions, permissions };
}

async function withService(
  run: (context: {
    cwd: string;
    events: JsonObject[];
    interactions: InteractionBroker;
    permissions: PermissionService;
  }) => Promise<void>,
): Promise<void> {
  const cwd = mkdtempSync(join(tmpdir(), "nabla-permission-service-"));
  try {
    const context = service({ cwd });
    await run({ cwd, ...context });
  } finally {
    rmSync(cwd, { recursive: true, force: true });
  }
}

test("builtin tools are allowed without approval", async () => {
  await withService(async ({ cwd, permissions }) => {
    const result = await permissions.authorizeTool(event("t1", "ask_user", {}), {
      cwd,
    });
    assert.equal(result, undefined);
  });
});

test("profile tool restrictions block before approval", async () => {
  await withService(async ({ cwd, permissions }) => {
    const result = await permissions.authorizeTool(event("t1", "bash", {
      command: "rm -rf /",
    }), {
      cwd,
      agent: { profile: "worker", profileConfig: workerProfile },
    });
    assert.equal(result?.block, true);
    assert.match(String(result?.reason), /not exposed to profile worker/u);
  });
});

test("plan mode mutation is denied without approval", async () => {
  const cwd = mkdtempSync(join(tmpdir(), "nabla-permission-service-"));
  try {
    const { permissions } = service({ planMode: true, cwd });
    const result = await permissions.authorizeTool(event("t1", "edit", {
      path: "a.ts",
    }), {
      cwd,
    });
    assert.equal(result?.block, true);
    assert.equal(result?.reason, "Denied by permission policy");
  } finally {
    rmSync(cwd, { recursive: true, force: true });
  }
});

test("unknown tools request approval with high risk and allow once passes", async () => {
  const root = mkdtempSync(join(tmpdir(), "nabla-permission-service-"));
  try {
    const { permissions, interactions } = service({ cwd: root });
    let requestId = "";
    const originalRequest = interactions.requestApproval.bind(interactions);
    interactions.requestApproval = ((
      request: Parameters<InteractionBroker["requestApproval"]>[0],
      signal: Parameters<InteractionBroker["requestApproval"]>[1],
      notify: Parameters<InteractionBroker["requestApproval"]>[2],
    ) => {
      requestId = request.requestId;
      return originalRequest(request, signal, notify);
    }) as InteractionBroker["requestApproval"];
    const approving = permissions.authorizeTool(
      event("t1", "custom_tool", { value: 1 }),
      { cwd: root },
    );
    const deadline = Date.now() + 1_000;
    while (!requestId && Date.now() < deadline) {
      await new Promise((resolve) => setTimeout(resolve, 10));
    }
    assert.ok(requestId, "approval request was announced");
    interactions.replyApproval(requestId, "allow_once");
    assert.equal(await approving, undefined);
    permissions.finishTool("t1", true);
    permissions.finishTool("t1", true);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("credential and outside-workspace paths are classified on the approval request", async () => {
  await withService(async ({ cwd, permissions, interactions }) => {
    const risks: string[] = [];
    const originalRequest = interactions.requestApproval.bind(interactions);
    interactions.requestApproval = ((
      request: Parameters<InteractionBroker["requestApproval"]>[0],
      signal: Parameters<InteractionBroker["requestApproval"]>[1],
      notify: Parameters<InteractionBroker["requestApproval"]>[2],
    ) => {
      risks.push(request.risk);
      return originalRequest(request, signal, notify);
    }) as InteractionBroker["requestApproval"];

    const credential = permissions.authorizeTool(
      event("t1", "edit", { path: ".ssh/id_rsa" }),
      { cwd },
    );
    const deadline = Date.now() + 1_000;
    while (risks.length < 1 && Date.now() < deadline) {
      await new Promise((resolve) => setTimeout(resolve, 10));
    }
    interactions.replyApproval("request-t1", "deny");
    await credential;

    const outside = permissions.authorizeTool(
      event("t2", "edit", { path: "../outside.txt" }),
      { cwd },
    );
    while (risks.length < 2 && Date.now() < deadline) {
      await new Promise((resolve) => setTimeout(resolve, 10));
    }
    interactions.replyApproval("request-t2", "deny");
    await outside;

    assert.ok(risks.includes("credential"));
    assert.ok(risks.includes("outside_workspace"));
  });
});

test("denied approvals block with the user-denied reason", async () => {
  await withService(async ({ cwd, permissions, interactions }) => {
    const pending = permissions.authorizeTool(
      event("t1", "bash", { command: "echo hi" }),
      { cwd },
    );
    const deadline = Date.now() + 1_000;
    while (Date.now() < deadline) {
      try {
        interactions.replyApproval("request-t1", "deny");
        break;
      } catch {
        await new Promise((resolve) => setTimeout(resolve, 10));
      }
    }
    const result = await pending;
    assert.equal(result?.block, true);
    assert.equal(result?.reason, "Denied by user");
  });
});

test("workspace rules snapshot, revoke, and clear round-trip", async () => {
  const root = mkdtempSync(join(tmpdir(), "nabla-permission-service-"));
  try {
    const { permissions } = service({ cwd: root });
    const initial = permissions.workspaceRules();
    assert.equal(initial.grants.length, 0);
    const cleared = permissions.clearWorkspaceRules();
    assert.equal(cleared.grants.length, 0);
    assert.doesNotThrow(() => permissions.revokeWorkspaceRule("missing"));
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
