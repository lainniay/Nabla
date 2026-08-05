import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { intersectGrantSets } from "./permissions/adapters/agent.ts";
import { ShellAdapter } from "./permissions/adapters/shell.ts";
import { createIntent } from "./permissions/adapters/tool-adapter.ts";
import { MemoryPermissionAuditLog } from "./permissions/audit-log.ts";
import { ApprovalBroker } from "./permissions/approvals/broker.ts";
import { OnceGrantStore } from "./permissions/approvals/once-store.ts";
import { SessionGrantStore } from "./permissions/approvals/session-store.ts";
import { WorkspaceGrantStore } from "./permissions/approvals/workspace-store.ts";
import { evaluatePermission } from "./permissions/evaluator.ts";
import { proposeGrantBundles } from "./permissions/grant-proposal.ts";
import { PermissionKernel } from "./permissions/kernel.ts";
import type {
  CapabilityMatcher,
  PermissionIntent,
  PermissionRule,
  ToolContext,
} from "./permissions/model.ts";
import { PolicyStore } from "./permissions/policy-store.ts";
import { planShell } from "./permissions/shell/planner.ts";
import {
  fileDigest,
  resolveWorkspaceIdentity,
} from "./permissions/workspace-identity.ts";

function context(workspace: string, sessionId = "session-1"): ToolContext {
  const identity = resolveWorkspaceIdentity(workspace);
  return {
    requestId: "request-1",
    toolCallId: "tool-1",
    sessionId,
    workspaceId: identity.id,
    cwd: workspace,
  };
}

function execIntent(workspace: string): PermissionIntent {
  const ctx = context(workspace);
  return createIntent(ctx, "bash", { command: "a" }, [{
    kind: "exec",
    executable: "a",
    argv: [],
    cwd: workspace,
    environment: {},
  }]);
}

function fixture() {
  const root = mkdtempSync(join(tmpdir(), "nabla-permissions-"));
  const home = join(root, "home");
  const workspace = join(root, "workspace");
  const other = join(root, "other");
  mkdirSync(home);
  mkdirSync(workspace);
  mkdirSync(other);
  return {
    root,
    home,
    workspace,
    other,
    cleanup: () => rmSync(root, { recursive: true, force: true }),
  };
}

test("policy precedence is deny > ask > allow regardless of specificity or order", () => {
  const value = fixture();
  try {
    const intent = execIntent(value.workspace);
    const matcher: CapabilityMatcher = {
      kind: "exec",
      executable: "a",
      argv: [],
    };
    const rules: PermissionRule[] = [
      { id: "deny", effect: "deny", matcher, source: "managed" },
      { id: "allow", effect: "allow", matcher, source: "user" },
      { id: "ask", effect: "ask", matcher, source: "session" },
    ];
    assert.equal(evaluatePermission(intent, rules).effect, "deny");
    assert.equal(evaluatePermission(intent, rules.slice(1)).effect, "ask");
    assert.equal(evaluatePermission(intent, [rules[1]!]).effect, "allow");
    assert.equal(evaluatePermission(intent, []).effect, "ask");
  } finally {
    value.cleanup();
  }
});

test("session grants do not leak across sessions or workspaces", () => {
  const value = fixture();
  try {
    const intent = execIntent(value.workspace);
    const store = new SessionGrantStore();
    const grant = proposeGrantBundles(intent).find((item) => item.scope === "session")!;
    store.add(grant);
    assert.equal(store.get("session-1", intent.workspaceId).length, 1);
    assert.equal(store.get("session-2", intent.workspaceId).length, 0);
    assert.equal(store.get("session-1", "another-workspace").length, 0);
  } finally {
    value.cleanup();
  }
});

test("workspace grants are identity scoped and invalidated by manifest changes", () => {
  const value = fixture();
  try {
    const packagePath = join(value.workspace, "package.json");
    writeFileSync(packagePath, "{\"scripts\":{\"test\":\"node --test\"}}\n");
    const identity = resolveWorkspaceIdentity(value.workspace);
    const ctx = { ...context(value.workspace), workspaceId: identity.id };
    const adapter = new ShellAdapter();
    const intent = adapter.normalize(ctx, { command: "npm test" });
    const bundle = proposeGrantBundles(intent, identity)
      .find((item) => item.scope === "workspace")!;
    const store = new WorkspaceGrantStore(value.home);
    store.add(bundle);
    assert.equal(store.get(identity).length, 1);
    assert.equal(store.get(resolveWorkspaceIdentity(value.other)).length, 0);
    writeFileSync(packagePath, "{\"scripts\":{\"test\":\"node changed.js\"}}\n");
    assert.equal(store.get(identity).length, 0);
  } finally {
    value.cleanup();
  }
});

test("legacy exact path and command approvals migrate while prefixes do not", () => {
  const value = fixture();
  try {
    mkdirSync(join(value.home, ".nabla"));
    writeFileSync(
      join(value.home, ".nabla", "approvals.json"),
      JSON.stringify({
        schemaVersion: 1,
        rules: [
          {
            workspace: value.workspace,
            toolName: "write",
            kind: "path",
            value: "file.txt",
            recursive: false,
          },
          {
            workspace: value.workspace,
            toolName: "bash",
            kind: "command",
            value: "cargo test",
            recursive: false,
          },
          {
            workspace: value.workspace,
            toolName: "bash",
            kind: "command_prefix",
            value: "npm test",
            recursive: false,
          },
        ],
      }),
    );
    const identity = resolveWorkspaceIdentity(value.workspace);
    const grants = new WorkspaceGrantStore(value.home).get(identity);
    assert.equal(grants.length, 2);
    assert.equal(
      grants.some((grant) =>
        grant.matchers.some((matcher) => matcher.kind === "opaque_shell_exact")),
      true,
    );
    assert.equal(
      grants.some((grant) =>
        grant.matchers.some(
          (matcher) =>
            matcher.kind === "opaque_shell_exact" && matcher.command === "npm test",
        )),
      false,
    );
  } finally {
    value.cleanup();
  }
});

test("allow once is bound to request, tool call, digest, session and workspace and consumes once", () => {
  const value = fixture();
  try {
    const intent = execIntent(value.workspace);
    const bundle = proposeGrantBundles(intent).find((item) => item.scope === "once")!;
    const store = new OnceGrantStore();
    store.put({
      requestId: "request-1",
      toolCallId: intent.toolCallId,
      intentDigest: intent.digest,
      sessionId: intent.sessionId,
      workspaceId: intent.workspaceId,
      bundle,
    });
    assert.ok(store.consume(intent, "request-1"));
    assert.equal(store.consume(intent, "request-1"), undefined);
    const changed = { ...intent, digest: "changed" };
    assert.equal(store.peek(changed, "request-1"), undefined);
  } finally {
    value.cleanup();
  }
});

test("file operation matchers cannot authorize a different operation", () => {
  const value = fixture();
  try {
    const ctx = context(value.workspace);
    const path = join(value.workspace, "file.txt");
    const read = createIntent(ctx, "read", { path }, [{
      kind: "file",
      operation: "read",
      path,
    }]);
    const write = createIntent(ctx, "write", { path }, [{
      kind: "file",
      operation: "write",
      path,
    }]);
    const rule: PermissionRule = {
      id: "read-only",
      effect: "allow",
      source: "user",
      matcher: { kind: "file", operation: "read", path },
    };
    assert.equal(evaluatePermission(read, [rule]).effect, "allow");
    assert.equal(evaluatePermission(write, [rule]).effect, "ask");
  } finally {
    value.cleanup();
  }
});

test("project policy can narrow but cannot add allow rules", () => {
  const store = new PolicyStore();
  const matcher: CapabilityMatcher = { kind: "exec", executable: "a" };
  store.setProject([
    { id: "project-allow", effect: "allow", matcher, source: "workspace" },
    { id: "project-deny", effect: "deny", matcher, source: "workspace" },
  ]);
  assert.deepEqual(store.all().map((rule) => rule.id), ["project-deny"]);
});

test("child grants are an intersection and cannot expand parent grants", () => {
  const read: CapabilityMatcher = {
    kind: "file",
    operation: "read",
    path: "/workspace",
    recursive: true,
  };
  const write: CapabilityMatcher = {
    kind: "file",
    operation: "write",
    path: "/workspace",
    recursive: true,
  };
  assert.deepEqual(
    intersectGrantSets(
      { matchers: [read] },
      { matchers: [read, write] },
      { matchers: [read] },
    ).matchers,
    [read],
  );
});

for (const [source, executables, fileOperations] of [
  ["a && b", ["a", "b"], []],
  ["a || b", ["a", "b"], []],
  ["a ; b", ["a", "b"], []],
  ["a\nb", ["a", "b"], []],
  ["a | b", ["a", "b"], []],
  ["a |& b", ["a", "b"], []],
  ["a & b", ["a", "b"], []],
  ["a > file", ["a"], ["write"]],
  ["a >> file", ["a"], ["append"]],
  ["a < file", ["a"], ["read"]],
  ["a 2> file", ["a"], ["write"]],
  ["(a && b)", ["a", "b"], []],
  ["cd dir && command", ["cd", "command"], []],
  ["bash -c 'a && b'", ["bash", "a", "b"], []],
  ["command $(other-command)", ["command", "other-command"], []],
] as const) {
  test(`shell planner creates independent atoms for ${JSON.stringify(source)}`, () => {
    const plan = planShell(source, "/workspace");
    assert.equal(plan.opaque, false);
    assert.deepEqual(
      plan.atoms.filter((atom) => atom.kind === "exec")
        .map((atom) => atom.kind === "exec" ? atom.executable : ""),
      executables,
    );
    assert.deepEqual(
      plan.atoms.filter((atom) => atom.kind === "file")
        .map((atom) => atom.kind === "file" ? atom.operation : ""),
      fileOperations,
    );
  });
}

test("unparsed and interpreter code become opaque and never implicit allow", () => {
  assert.equal(planShell("python -c 'print(1)'", "/workspace").opaque, true);
  assert.equal(planShell("a `dynamic`", "/workspace").opaque, true);
  assert.equal(planShell("bash -c \"$SCRIPT\"", "/workspace").opaque, true);
  assert.equal(planShell("cat src/*.ts", "/workspace").opaque, true);
});

test("environment assignments are attached to the executable capability", () => {
  const plan = planShell("MODE=test command argument", "/workspace");
  const command = plan.atoms.find((atom) => atom.kind === "exec");
  assert.equal(command?.kind, "exec");
  assert.deepEqual(
    command?.kind === "exec" ? command.environment : undefined,
    { MODE: "test" },
  );
});

test("kernel preflights every atom and denied compound commands never reach approval", async () => {
  const value = fixture();
  try {
    const identity = resolveWorkspaceIdentity(value.workspace);
    const ctx = { ...context(value.workspace), workspaceId: identity.id };
    const intent = new ShellAdapter().normalize(ctx, { command: "safe && denied" });
    const policies = new PolicyStore();
    policies.setManaged([{
      id: "deny-second",
      effect: "deny",
      source: "managed",
      matcher: { kind: "exec", executable: "denied" },
    }]);
    const audit = new MemoryPermissionAuditLog();
    const kernel = new PermissionKernel(
      policies,
      new ApprovalBroker(
        new OnceGrantStore(),
        new SessionGrantStore(),
        new WorkspaceGrantStore(value.home),
      ),
      audit,
    );
    let prompted = false;
    const result = await kernel.authorize(
      ctx.requestId,
      intent,
      identity,
      async () => {
        prompted = true;
        return "allow_once";
      },
    );
    assert.equal(result.evaluation.effect, "deny");
    assert.equal(prompted, false);
    assert.equal(audit.entries[0]?.matchedRuleIds[0], "deny-second");
  } finally {
    value.cleanup();
  }
});

test("an unapproved pipeline is denied before any execution can start", async () => {
  const value = fixture();
  try {
    const identity = resolveWorkspaceIdentity(value.workspace);
    const ctx = { ...context(value.workspace), workspaceId: identity.id };
    const intent = new ShellAdapter().normalize(ctx, { command: "allowed | pending" });
    const policies = new PolicyStore();
    policies.setBuiltin([{
      id: "allow-first",
      effect: "allow",
      source: "builtin",
      matcher: { kind: "exec", executable: "allowed" },
    }]);
    const kernel = new PermissionKernel(
      policies,
      new ApprovalBroker(
        new OnceGrantStore(),
        new SessionGrantStore(),
        new WorkspaceGrantStore(value.home),
      ),
      new MemoryPermissionAuditLog(),
    );
    let executions = 0;
    const authorization = await kernel.authorize(
      ctx.requestId,
      intent,
      identity,
      async () => "deny",
    );
    if (authorization.evaluation.effect === "allow") executions += 1;
    assert.equal(authorization.evaluation.effect, "ask");
    assert.equal(authorization.decision, "deny");
    assert.equal(executions, 0);
  } finally {
    value.cleanup();
  }
});

test("execution rejects post-approval cwd, argv, and environment changes", async () => {
  const value = fixture();
  try {
    const identity = resolveWorkspaceIdentity(value.workspace);
    const adapter = new ShellAdapter();
    const ctx = { ...context(value.workspace), workspaceId: identity.id };
    const approved = adapter.normalize(ctx, {
      command: "tool argument",
      environment: { MODE: "approved" },
    });
    const kernel = new PermissionKernel(
      new PolicyStore(),
      new ApprovalBroker(
        new OnceGrantStore(),
        new SessionGrantStore(),
        new WorkspaceGrantStore(value.home),
      ),
      new MemoryPermissionAuditLog(),
    );
    const changedArgv = adapter.normalize(ctx, {
      command: "tool changed",
      environment: { MODE: "approved" },
    });
    const changedEnvironment = adapter.normalize(ctx, {
      command: "tool argument",
      environment: { MODE: "changed" },
    });
    const changedCwd = adapter.normalize(
      { ...ctx, cwd: value.other },
      { command: "tool argument", environment: { MODE: "approved" } },
    );
    for (const changed of [changedArgv, changedEnvironment, changedCwd]) {
      const authorization = await kernel.authorize(
        ctx.requestId,
        approved,
        identity,
        async () => "allow_once",
      );
      assert.equal(kernel.consumeForExecution(authorization, changed), false);
      assert.equal(kernel.consumeForExecution(authorization, approved), false);
    }
    const valid = await kernel.authorize(
      ctx.requestId,
      approved,
      identity,
      async () => "allow_once",
    );
    assert.equal(kernel.consumeForExecution(valid, approved), true);
    assert.equal(kernel.consumeForExecution(valid, approved), false);
  } finally {
    value.cleanup();
  }
});

test("workspace recreation changes identity and invalidates old grants", () => {
  const value = fixture();
  try {
    const before = resolveWorkspaceIdentity(value.workspace);
    const intent = createIntent(
      { ...context(value.workspace), workspaceId: before.id },
      "write",
      { path: join(value.workspace, "file.txt") },
      [{
        kind: "file",
        operation: "write",
        path: join(value.workspace, "file.txt"),
      }],
    );
    const store = new WorkspaceGrantStore(value.home);
    store.add(
      proposeGrantBundles(intent, before)
        .find((bundle) => bundle.scope === "workspace")!,
    );
    rmSync(value.workspace, { recursive: true });
    mkdirSync(value.workspace);
    const after = resolveWorkspaceIdentity(value.workspace);
    assert.notEqual(after.id, before.id);
    assert.equal(store.get(after).length, 0);
  } finally {
    value.cleanup();
  }
});

test("automatic allows are audited to a concrete policy rule", async () => {
  const value = fixture();
  try {
    const identity = resolveWorkspaceIdentity(value.workspace);
    const intent = execIntent(value.workspace);
    const policies = new PolicyStore();
    policies.setBuiltin([{
      id: "builtin-exec-a",
      effect: "allow",
      source: "builtin",
      matcher: { kind: "exec", executable: "a" },
    }]);
    const audit = new MemoryPermissionAuditLog();
    const kernel = new PermissionKernel(
      policies,
      new ApprovalBroker(
        new OnceGrantStore(),
        new SessionGrantStore(),
        new WorkspaceGrantStore(value.home),
      ),
      audit,
    );
    const result = await kernel.authorize(
      "request-1",
      intent,
      identity,
      async () => "deny",
    );
    assert.equal(result.evaluation.effect, "allow");
    assert.deepEqual(audit.entries[0]?.matchedRuleIds, ["builtin-exec-a"]);
  } finally {
    value.cleanup();
  }
});

test("subagent authorization cannot create workspace grants", async () => {
  const value = fixture();
  try {
    const identity = resolveWorkspaceIdentity(value.workspace);
    const intent = execIntent(value.workspace);
    const workspaceStore = new WorkspaceGrantStore(value.home);
    const kernel = new PermissionKernel(
      new PolicyStore(),
      new ApprovalBroker(
        new OnceGrantStore(),
        new SessionGrantStore(),
        workspaceStore,
      ),
      new MemoryPermissionAuditLog(),
    );
    const result = await kernel.authorize(
      "request-1",
      intent,
      identity,
      async () => "allow_workspace",
      undefined,
      [],
      false,
    );
    assert.equal(result.decision, "deny");
    assert.equal(workspaceStore.get(identity).length, 0);
  } finally {
    value.cleanup();
  }
});

test("an ask policy requires approval every time but accepts the current decision", async () => {
  const value = fixture();
  try {
    const identity = resolveWorkspaceIdentity(value.workspace);
    const intent = execIntent(value.workspace);
    const policies = new PolicyStore();
    policies.setManaged([{
      id: "always-ask",
      effect: "ask",
      source: "managed",
      matcher: { kind: "exec", executable: "a" },
    }]);
    const kernel = new PermissionKernel(
      policies,
      new ApprovalBroker(
        new OnceGrantStore(),
        new SessionGrantStore(),
        new WorkspaceGrantStore(value.home),
      ),
      new MemoryPermissionAuditLog(),
    );
    let prompts = 0;
    const requester = async () => {
      prompts += 1;
      return "allow_once" as const;
    };
    const first = await kernel.authorize(
      "request-1",
      intent,
      identity,
      requester,
    );
    assert.equal(first.evaluation.effect, "ask");
    assert.equal(kernel.consumeForExecution(first, intent), true);
    const second = await kernel.authorize(
      "request-2",
      { ...intent, toolCallId: "tool-2" },
      identity,
      requester,
    );
    assert.equal(second.evaluation.effect, "ask");
    assert.equal(prompts, 2);
  } finally {
    value.cleanup();
  }
});

test("file digest helper observes manifest changes", () => {
  const value = fixture();
  try {
    const path = join(value.workspace, "package.json");
    writeFileSync(path, "one");
    const before = fileDigest(path);
    writeFileSync(path, "two");
    assert.notEqual(fileDigest(path), before);
  } finally {
    value.cleanup();
  }
});
