import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import type { ToolCallEvent } from "@earendil-works/pi-coding-agent";

import { InteractionBroker } from "../interactions/interaction-broker.ts";
import type { AgentProfile } from "../subagents/profile-model.ts";
import type { SandboxCapability } from "./execution/sandbox-capability.ts";
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
  sandbox?: SandboxCapability;
} = {}) {
  const events: JsonObject[] = [];
  const interactions = new InteractionBroker();
  const capability =
    options.sandbox ?? {
      mode: "degraded" as const,
      backend: "none" as const,
      supportsFilesystemIsolation: false,
      supportsNetworkIsolation: false,
    };
  const permissions = new PermissionService(
    interactions,
    (event) => events.push(event),
    { current: () => options.planMode ?? false },
    () => options.connected ?? true,
    {
      sessionId: () => "session-1",
      cwd: () => options.cwd ?? "/workspace",
    },
    { capability: () => capability },
  );
  return { events, interactions, permissions };
}

async function withService(
  options:
    | Parameters<typeof service>[0]
    | ((context: {
        cwd: string;
        events: JsonObject[];
        interactions: InteractionBroker;
        permissions: PermissionService;
      }) => Promise<void>),
  run?: (context: {
    cwd: string;
    events: JsonObject[];
    interactions: InteractionBroker;
    permissions: PermissionService;
  }) => Promise<void>,
): Promise<void> {
  const actualOptions = typeof options === "function" ? {} : options;
  const actualRun = typeof options === "function" ? options : run;
  if (!actualRun) throw new Error("withService requires a run callback");
  const cwd = mkdtempSync(join(tmpdir(), "nabla-permission-service-"));
  try {
    const context = service({ ...actualOptions, cwd });
    await actualRun({ cwd, ...context });
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

test("workspace read-only tools are allowed without approval", async () => {
  await withService(async ({ cwd, permissions }) => {
    const read = await permissions.authorizeTool(
      event("t1", "read", { path: "a.ts" }),
      { cwd },
    );
    assert.equal(read, undefined);
    const find = await permissions.authorizeTool(
      event("t2", "find", { pattern: "*.ts", path: "." }),
      { cwd },
    );
    assert.equal(find, undefined);
    const patternOnly = await permissions.authorizeTool(
      event("t3", "find", { pattern: "*.md" }),
      { cwd },
    );
    assert.equal(patternOnly, undefined);
  });
});

test("credential reads stay denied inside the workspace", async () => {
  await withService(async ({ cwd, permissions }) => {
    const result = await permissions.authorizeTool(
      event("t1", "read", { path: ".ssh/id_rsa" }),
      { cwd },
    );
    assert.equal(result?.block, true);
    assert.equal(result?.reason, "Denied by permission policy");
  });
});

test("outside-workspace reads still require approval", async () => {
  await withService(async ({ cwd, permissions, interactions }) => {
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
    const pending = permissions.authorizeTool(
      event("t1", "read", { path: "../outside.txt" }),
      { cwd },
    );
    const deadline = Date.now() + 1_000;
    while (!requestId && Date.now() < deadline) {
      await new Promise((resolve) => setTimeout(resolve, 10));
    }
    assert.ok(requestId, "approval request was announced");
    interactions.replyApproval(requestId, "deny");
    const result = await pending;
    assert.equal(result?.block, true);
  });
});

test("mutation tools still require approval", async () => {
  await withService(async ({ cwd, permissions, interactions }) => {
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
    const pending = permissions.authorizeTool(
      event("t1", "edit", { path: "a.ts" }),
      { cwd },
    );
    const deadline = Date.now() + 1_000;
    while (!requestId && Date.now() < deadline) {
      await new Promise((resolve) => setTimeout(resolve, 10));
    }
    assert.ok(requestId, "approval request was announced");
    interactions.replyApproval(requestId, "deny");
    const result = await pending;
    assert.equal(result?.block, true);
  });
});

test("workspace read-only git and benign pipeline tools are allowed without approval", async () => {
  await withService(async ({ cwd, permissions }) => {
    const command = [
      `git -C ${cwd} log --oneline -10 2>/dev/null`,
      'echo "---"',
      `git -C ${cwd} status --short 2>/dev/null | head -30`,
    ].join("; ");
    const result = await permissions.authorizeTool(
      event("t1", "bash", { command }),
      { cwd },
    );
    assert.equal(result, undefined);
    const ls = await permissions.authorizeTool(
      event("t2", "bash", {
        command: "ls src/ agent-host/src/ agent-host/ 2>/dev/null",
      }),
      { cwd },
    );
    assert.equal(ls, undefined);
    const wc = await permissions.authorizeTool(
      event("t3", "bash", {
        command: [
          "wc -l src/*.rs src/app/*.rs 2>/dev/null | tail -5",
          'echo "==="',
          "ls agent-host/src/features agent-host/src/permissions 2>/dev/null | head -80",
        ].join("; "),
      }),
      { cwd },
    );
    assert.equal(wc, undefined);
    const cd = await permissions.authorizeTool(
      event("t4", "bash", {
        command: `cd ${cwd} && echo ok`,
      }),
      { cwd },
    );
    assert.equal(cd, undefined);
    const findXargs = await permissions.authorizeTool(
      event("t5", "bash", {
        command: [
          "git log --oneline -15 2>/dev/null",
          'echo ---',
          "git status --short 2>/dev/null | head -20",
          "find src -name '*.rs' | xargs wc -l 2>/dev/null | tail -1",
          "find agent-host/src -name '*.ts' | xargs wc -l 2>/dev/null | tail -1",
        ].join("; "),
      }),
      { cwd },
    );
    assert.equal(findXargs, undefined);
  });
});

test("non-readonly git commands still require approval", async () => {
  await withService(async ({ cwd, permissions, interactions }) => {
    for (const [index, source] of [
      "git push",
      "git -C /outside log",
      "git status > out.txt",
      "git -c core.pager=x log",
      "git status | cat",
      "ls /etc",
      "wc /etc/passwd",
      "cd /etc && echo hi",
      `cd ${cwd} && cargo test 2>&1 | tail -40`,
      "find /etc -name '*.conf'",
      "find . -exec rm {} \\;",
      "echo hi | xargs rm",
      "echo hi | xargs cat",
    ].entries()) {
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
      const pending = permissions.authorizeTool(
        event(`t${index}`, "bash", { command: source }),
        { cwd },
      );
      const deadline = Date.now() + 1_000;
      while (!requestId && Date.now() < deadline) {
        await new Promise((resolve) => setTimeout(resolve, 10));
      }
      assert.ok(requestId, `approval was announced for: ${source}`);
      interactions.replyApproval(requestId, "deny");
      const result = await pending;
      assert.equal(result?.block, true, source);
    }
  });
});

test("enforced sandbox auto-allows non-dangerous bash commands", async () => {
  const sandbox: SandboxCapability = {
    mode: "enforced",
    backend: "seatbelt",
    supportsFilesystemIsolation: true,
    supportsNetworkIsolation: true,
  };
  await withService({ sandbox }, async ({ cwd, permissions }) => {
    for (const source of [
      "cargo test",
      "cargo check",
      "cargo build --release",
      "npm test",
      "node script.js",
    ]) {
      const result = await permissions.authorizeTool(
        event("t1", "bash", { command: source }),
        { cwd },
      );
      assert.equal(result, undefined, source);
    }
  });
});

test("enforced sandbox still asks for dangerous or network commands", async () => {
  const sandbox: SandboxCapability = {
    mode: "enforced",
    backend: "seatbelt",
    supportsFilesystemIsolation: true,
    supportsNetworkIsolation: true,
  };
  await withService({ sandbox }, async ({ cwd, permissions, interactions }) => {
    for (const [index, source] of [
      "rm -rf /",
      "git push",
      "cargo publish",
      "curl example.com",
      "find . -exec rm {} \\;",
      "echo hi | xargs rm",
    ].entries()) {
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
      const pending = permissions.authorizeTool(
        event(`t${index}`, "bash", { command: source }),
        { cwd },
      );
      const deadline = Date.now() + 1_000;
      while (!requestId && Date.now() < deadline) {
        await new Promise((resolve) => setTimeout(resolve, 10));
      }
      assert.ok(requestId, `approval was announced for: ${source}`);
      interactions.replyApproval(requestId, "deny");
      const result = await pending;
      assert.equal(result?.block, true, source);
    }
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
      event("t1", "bash", { command: "cat file" }),
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
