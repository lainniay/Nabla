import assert from "node:assert/strict";
import test from "node:test";

import type { BashOperations, ToolDefinition } from "@earendil-works/pi-coding-agent";

import type { PermissionService } from "../features/permissions/permission-service.ts";
import type { RustSandboxBackend } from "../permissions/execution/rust-sandbox-backend.ts";
import type { SandboxExecutionProfile } from "../permissions/execution/sandbox-profile.ts";
import { createNablaBashTool } from "./create-nabla-bash-tool.ts";

const profile: SandboxExecutionProfile = {
  mode: "enforced",
  backend: "native",
  filesystem: { readWrite: ["/workspace"], denyRead: [], denyWrite: [] },
  network: "blocked",
};

test("bash tool authorizes once, runs through sandbox operations, and finishes once", async () => {
  const calls: string[] = [];
  const permissions = {
    authorizeBash: async () => {
      calls.push("authorize");
      return {
        authorizationId: "a1",
        decision: "allow" as const,
        commandDigest: "d",
        profile,
      };
    },
    finishBash: (_toolCallId: string, succeeded: boolean) => {
      calls.push(`finish:${String(succeeded)}`);
    },
  } as unknown as PermissionService;
  const sandboxBackend = {
    operationsFor: (used: SandboxExecutionProfile) => {
      assert.equal(used, profile);
      calls.push("operations");
      return {
        exec: async (
          command: string,
          _cwd: string,
          _options: { onData: (data: Buffer) => void },
        ) => {
          calls.push(`exec:${command}`);
          return { exitCode: 0 };
        },
      } as BashOperations;
    },
  } as unknown as RustSandboxBackend;

  const tool = createNablaBashTool("/workspace", {
    permissions,
    sandboxBackend,
  }) as ToolDefinition;
  const result = await tool.execute(
    "t1",
    { command: "echo hi" },
    undefined,
    undefined,
    {
      sessionManager: {
        getSessionId: () => "session-1",
        getSessionFile: () => undefined,
      },
    } as never,
  );

  assert.deepEqual(calls, ["authorize", "operations", "exec:echo hi", "finish:true"]);
  assert.equal(result.content[0]?.type, "text");
});

test("denied authorization throws and never executes", async () => {
  const permissions = {
    authorizeBash: async () => ({
      authorizationId: "denied",
      decision: "deny" as const,
      reason: "Denied by user",
      commandDigest: "",
      profile,
    }),
    finishBash: () => {
      assert.fail("finishBash must not be called for a denied call");
    },
  } as unknown as PermissionService;
  const tool = createNablaBashTool("/workspace", {
    permissions,
    sandboxBackend: {} as RustSandboxBackend,
  }) as ToolDefinition;

  await assert.rejects(
    tool.execute(
      "t1",
      { command: "echo hi" },
      undefined,
      undefined,
      {
        sessionManager: {
          getSessionId: () => "session-1",
          getSessionFile: () => undefined,
        },
      } as never,
    ),
    /Denied by user/u,
  );
});
