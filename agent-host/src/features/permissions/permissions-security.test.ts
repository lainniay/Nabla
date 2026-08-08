import assert from "node:assert/strict";
import {
  mkdirSync,
  mkdtempSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { ShellAdapter } from "./adapters/shell.ts";
import { createIntent } from "./adapters/tool-adapter.ts";
import { MemoryPermissionAuditLog } from "./audit-log.ts";
import { ApprovalBroker } from "./approvals/broker.ts";
import { OnceGrantStore } from "./approvals/once-store.ts";
import { SessionGrantStore } from "./approvals/session-store.ts";
import { WorkspaceGrantStore } from "./approvals/workspace-store.ts";
import { evaluatePermission } from "./evaluator.ts";
import { proposeGrantBundles } from "./grant-proposal.ts";
import { PermissionKernel } from "./kernel.ts";
import type {
  GrantBundle,
  PermissionIntent,
  ToolContext,
} from "./model.ts";
import { PolicyStore } from "./policy-store.ts";
import {
  buildCredentialDenyRules,
  buildReadOnlyBashRules,
} from "./policy/builtin.ts";
import { digestValue } from "./shell/digest.ts";
import { resolveWorkspaceIdentity } from "./workspace-identity.ts";

// Security regression tests for the permission control plane:
// - an approval only resolves the policy state the user saw;
// - execution re-validates current policy and grant validity;
// - proposals and saved grants only cover atoms that actually needed approval;
// - credential-path reads through shell tools are uniformly planned;
// - legacy shell_digest grants are inert.

function fixture() {
  const root = mkdtempSync(join(tmpdir(), "nabla-security-"));
  const home = join(root, "home");
  const workspace = join(root, "workspace");
  mkdirSync(home);
  mkdirSync(workspace);
  return {
    root,
    home,
    workspace,
    cleanup: () => rmSync(root, { recursive: true, force: true }),
  };
}

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

test("session/workspace approvals are rejected when policy changes during approval", async () => {
  const value = fixture();
  try {
    for (const decision of ["allow_session", "allow_workspace"] as const) {
      const identity = resolveWorkspaceIdentity(value.workspace);
      const intent = execIntent(value.workspace);
      const policies = new PolicyStore();
      const sessionStore = new SessionGrantStore();
      const workspaceStore = new WorkspaceGrantStore(value.home);
      const audit = new MemoryPermissionAuditLog();
      const kernel = new PermissionKernel(
        policies,
        new ApprovalBroker(
          new OnceGrantStore(),
          sessionStore,
          workspaceStore,
        ),
        audit,
      );
      const authorization = await kernel.authorize(
        "request-1",
        intent,
        identity,
        async () => {
          policies.setManaged([{
            id: "ask-after-approval",
            effect: "ask",
            source: "managed",
            matcher: { kind: "exec", executable: "a" },
          }]);
          return decision;
        },
      );
      assert.equal(authorization.decision, "deny");
      assert.equal(authorization.deniedReason, "policy_changed");
      assert.equal(authorization.evaluation.effect, "ask");
      assert.equal(kernel.consume(authorization, intent), false);
      assert.equal(sessionStore.get(intent.sessionId, intent.workspaceId).length, 0);
      assert.equal(workspaceStore.get(identity).length, 0);
      assert.equal(audit.entries.some((entry) => entry.outcome === "denied"), true);
    }
  } finally {
    value.cleanup();
  }
});

test("kernel rejects a grant decision when policy changed to deny during approval", async () => {
  const value = fixture();
  try {
    const identity = resolveWorkspaceIdentity(value.workspace);
    const intent = execIntent(value.workspace);
    const policies = new PolicyStore();
    const kernel = new PermissionKernel(
      policies,
      new ApprovalBroker(
        new OnceGrantStore(),
        new SessionGrantStore(),
        new WorkspaceGrantStore(value.home),
      ),
      new MemoryPermissionAuditLog(),
    );
    const authorization = await kernel.authorize(
      "request-1",
      intent,
      identity,
      async () => {
        policies.setManaged([{
          id: "deny-after-approval",
          effect: "deny",
          source: "managed",
          matcher: { kind: "exec", executable: "a" },
        }]);
        return "allow_workspace";
      },
    );
    assert.equal(authorization.decision, "deny");
    assert.equal(authorization.deniedReason, "policy_changed");
    assert.equal(authorization.evaluation.effect, "deny");
    assert.equal(kernel.consume(authorization, intent), false);
  } finally {
    value.cleanup();
  }
});

test("ask policy with unchanged revision accepts the approved decision", async () => {
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
    for (const decision of ["allow_once", "allow_session", "allow_workspace"] as const) {
      const workspaceStore = new WorkspaceGrantStore(value.home);
      const kernel = new PermissionKernel(
        policies,
        new ApprovalBroker(
          new OnceGrantStore(),
          new SessionGrantStore(),
          workspaceStore,
        ),
        new MemoryPermissionAuditLog(),
      );
      const authorization = await kernel.authorize(
        "request-1",
        intent,
        identity,
        async () => decision,
      );
      assert.equal(authorization.decision, decision);
      assert.equal(authorization.evaluation.effect, "ask");
      assert.equal(kernel.consume(authorization, intent), true);
    }
  } finally {
    value.cleanup();
  }
});

test("consume re-evaluates policy after authorization", async () => {
  const value = fixture();
  try {
    const identity = resolveWorkspaceIdentity(value.workspace);
    const intent = execIntent(value.workspace);
    const policies = new PolicyStore();
    const kernel = new PermissionKernel(
      policies,
      new ApprovalBroker(
        new OnceGrantStore(),
        new SessionGrantStore(),
        new WorkspaceGrantStore(value.home),
      ),
      new MemoryPermissionAuditLog(),
    );
    const authorization = await kernel.authorize(
      "request-1",
      intent,
      identity,
      async () => "allow_workspace",
    );
    assert.equal(authorization.evaluation.effect, "allow");
    policies.setManaged([{
      id: "ask-after-authorize",
      effect: "ask",
      source: "managed",
      matcher: { kind: "exec", executable: "a" },
    }]);
    assert.equal(kernel.consume(authorization, intent), false);
  } finally {
    value.cleanup();
  }
});

test("once grants are invalidated when policy changes before execution", async () => {
  const value = fixture();
  try {
    const identity = resolveWorkspaceIdentity(value.workspace);
    const intent = execIntent(value.workspace);
    const policies = new PolicyStore();
    const onceStore = new OnceGrantStore();
    const kernel = new PermissionKernel(
      policies,
      new ApprovalBroker(
        onceStore,
        new SessionGrantStore(),
        new WorkspaceGrantStore(value.home),
      ),
      new MemoryPermissionAuditLog(),
    );
    let prompts = 0;
    const approveOnce = async (): Promise<"allow_once"> => {
      prompts += 1;
      return "allow_once";
    };

    const first = await kernel.authorize(
      "request-1",
      intent,
      identity,
      approveOnce,
    );
    assert.equal(first.decision, "allow_once");
    assert.equal(kernel.consume(first, intent), true);

    const second = await kernel.authorize(
      "request-2",
      intent,
      identity,
      approveOnce,
    );
    assert.equal(second.decision, "allow_once");
    policies.setManaged([{
      id: "ask-after-approval",
      effect: "ask",
      source: "managed",
      matcher: { kind: "exec", executable: "a" },
    }]);
    assert.equal(kernel.consume(second, intent), false);
    assert.equal(onceStore.peek(intent, "request-2"), undefined);

    const third = await kernel.authorize(
      "request-3",
      intent,
      identity,
      approveOnce,
    );
    assert.equal(third.decision, "allow_once");
    assert.equal(prompts, 3);
  } finally {
    value.cleanup();
  }
});

test("workspace grants are unavailable to subagent contexts", async () => {
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
    let workspaceProposal = false;
    const authorization = await kernel.authorize(
      "request-1",
      intent,
      identity,
      async ({ proposals }) => {
        workspaceProposal = proposals.some(
          (bundle) => bundle.scope === "workspace",
        );
        return "allow_workspace";
      },
      undefined,
      [],
      false,
    );
    assert.equal(workspaceProposal, false);
    assert.equal(authorization.decision, "deny");
    assert.equal(authorization.deniedReason, undefined);
    assert.equal(workspaceStore.get(identity).length, 0);
  } finally {
    value.cleanup();
  }
});

test("an approved grant stays executable while policy is unchanged even if a rule still asks", async () => {
  const value = fixture();
  try {
    const identity = resolveWorkspaceIdentity(value.workspace);
    const intent = execIntent(value.workspace);
    const kernel = new PermissionKernel(
      new PolicyStore(),
      new ApprovalBroker(
        new OnceGrantStore(),
        new SessionGrantStore(),
        new WorkspaceGrantStore(value.home),
      ),
      new MemoryPermissionAuditLog(),
    );
    const authorization = await kernel.authorize(
      "request-1",
      intent,
      identity,
      async () => "allow_workspace",
      undefined,
      [{
        id: "always-ask",
        effect: "ask",
        source: "managed",
        matcher: { kind: "exec", executable: "a" },
      }],
    );
    assert.equal(authorization.decision, "allow_workspace");
    assert.equal(authorization.evaluation.effect, "ask");
    assert.equal(kernel.consume(authorization, intent), true);
  } finally {
    value.cleanup();
  }
});

test("workspace grant invalidation blocks execution", async () => {
  const value = fixture();
  try {
    const packagePath = join(value.workspace, "package.json");
    writeFileSync(packagePath, "{\"scripts\":{\"test\":\"node --test\"}}\n");
    const identity = resolveWorkspaceIdentity(value.workspace);
    const ctx = { ...context(value.workspace), workspaceId: identity.id };
    const intent = new ShellAdapter().normalize(ctx, { command: "npm test" });
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
    const authorization = await kernel.authorize(
      ctx.requestId,
      intent,
      identity,
      async () => "allow_workspace",
    );
    assert.equal(authorization.evaluation.effect, "allow");
    writeFileSync(packagePath, "{\"scripts\":{\"test\":\"node changed.js\"}}\n");
    assert.equal(workspaceStore.get(identity).length, 0);
    assert.equal(kernel.consume(authorization, intent), false);
  } finally {
    value.cleanup();
  }
});

test("workspace grants only persist atoms that needed approval", async () => {
  const value = fixture();
  try {
    const identity = resolveWorkspaceIdentity(value.workspace);
    const ctx = { ...context(value.workspace), workspaceId: identity.id };
    const intent = new ShellAdapter().normalize(ctx, {
      command: "safe && danger",
    });
    const policies = new PolicyStore();
    policies.setBuiltin([{
      id: "builtin-safe",
      effect: "allow",
      source: "builtin",
      matcher: { kind: "exec", executable: "safe" },
    }]);
    const workspaceStore = new WorkspaceGrantStore(value.home);
    const kernel = new PermissionKernel(
      policies,
      new ApprovalBroker(
        new OnceGrantStore(),
        new SessionGrantStore(),
        workspaceStore,
      ),
      new MemoryPermissionAuditLog(),
    );
    const authorization = await kernel.authorize(
      ctx.requestId,
      intent,
      identity,
      async () => "allow_workspace",
    );
    assert.equal(authorization.decision, "allow_workspace");
    const grants = workspaceStore.get(identity);
    assert.equal(grants.length, 1);
    assert.equal(
      grants[0]!.matchers.some(
        (matcher) => matcher.kind === "exec" && matcher.executable === "safe",
      ),
      false,
    );
    assert.equal(
      grants[0]!.matchers.some(
        (matcher) => matcher.kind === "exec" && matcher.executable === "danger",
      ),
      true,
    );
    const afterPolicyChange = evaluatePermission(intent, [], grants);
    assert.equal(afterPolicyChange.effect, "ask");
    assert.equal(
      afterPolicyChange.atoms.find(
        (atom) => atom.atom.kind === "exec" && atom.atom.executable === "safe",
      )?.effect,
      "ask",
    );
    assert.equal(
      afterPolicyChange.atoms.find(
        (atom) =>
          atom.atom.kind === "exec" && atom.atom.executable === "danger",
      )?.effect,
      "allow",
    );
  } finally {
    value.cleanup();
  }
});

test("credential reads via tail are denied like head", () => {
  const value = fixture();
  try {
    const shell = new ShellAdapter();
    const ctx = context(value.workspace);
    const tail = shell.normalize(ctx, {
      command: "tail -n 5 /Users/test/.ssh/config",
    });
    const head = shell.normalize(ctx, {
      command: "head -n 5 /Users/test/.ssh/config",
    });
    assert.equal(tail.atoms.some((atom) => atom.kind === "file"), true);
    assert.equal(head.atoms.some((atom) => atom.kind === "file"), true);
    const tailRules = [
      ...buildCredentialDenyRules(tail),
      ...buildReadOnlyBashRules(shell.analysis(tail), tail),
    ];
    const headRules = [
      ...buildCredentialDenyRules(head),
      ...buildReadOnlyBashRules(shell.analysis(head), head),
    ];
    assert.equal(evaluatePermission(head, headRules).effect, "deny");
    assert.equal(evaluatePermission(tail, tailRules).effect, "deny");
  } finally {
    value.cleanup();
  }
});

test("legacy shell_digest grants never match", () => {
  const value = fixture();
  try {
    mkdirSync(join(value.workspace, "sub"));
    const identity = resolveWorkspaceIdentity(value.workspace);
    const shell = new ShellAdapter();
    const rootCtx = { ...context(value.workspace), workspaceId: identity.id };
    const subCtx = { ...rootCtx, cwd: join(value.workspace, "sub") };
    const rootIntent = shell.normalize(rootCtx, { command: "cat config.json" });
    const subIntent = shell.normalize(subCtx, { command: "cat config.json" });
    assert.notDeepEqual(rootIntent.atoms, subIntent.atoms);
    const grant: GrantBundle = {
      scope: "workspace",
      workspaceId: identity.id,
      matchers: [{
        kind: "shell_digest",
        digest: digestValue({ command: "cat config.json" }),
      }],
    };
    assert.equal(evaluatePermission(rootIntent, [], [grant]).effect, "ask");
    assert.equal(evaluatePermission(subIntent, [], [grant]).effect, "ask");
  } finally {
    value.cleanup();
  }
});

test("workspace grants are not bound to a principal (documented limitation)", () => {
  const value = fixture();
  try {
    const identity = resolveWorkspaceIdentity(value.workspace);
    const intent = execIntent(value.workspace);
    const grant = proposeGrantBundles(intent, identity)
      .find((bundle) => bundle.scope === "workspace")!;
    const reviewerIntent = { ...intent, sessionId: "session-reviewer" };
    const evaluation = evaluatePermission(reviewerIntent, [], [grant]);
    assert.equal(evaluation.effect, "allow");
    assert.equal(evaluation.atoms[0]?.grants[0]?.sessionId, undefined);
  } finally {
    value.cleanup();
  }
});
