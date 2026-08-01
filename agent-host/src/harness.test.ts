import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import {
  GoalStore,
  agentPermissionEffect,
  commandAllowedByLease,
  filterContextFilesByTrust,
  loadHarnessConfig,
  pathAllowedByLease,
  saveWorkspaceTrust,
} from "./harness.ts";
import type { PlanArtifactV2 } from "./plan.ts";

function fixture() {
  const root = mkdtempSync(join(tmpdir(), "nabla-goal-"));
  const cwd = join(root, "workspace");
  mkdirSync(cwd);
  let timestamp = 0;
  return {
    root,
    cwd,
    create: () =>
      new GoalStore({
        cwd,
        sessionId: "session-1",
        rootDir: root,
        createId: () => "goal-1",
        now: () => `2026-01-01T00:00:${String(timestamp++).padStart(2, "0")}Z`,
      }),
  };
}

const spec = {
  summary: "Implement the requested feature",
  acceptanceCriteria: ["npm test"],
  allowedTools: ["read", "edit", "write", "bash"],
  allowedPaths: ["."],
  allowedCommands: ["npm test"],
  tasks: [],
};

const sourcePlan: PlanArtifactV2 = {
  schemaVersion: 2,
  id: "plan-source",
  revision: 3,
  status: "submitted",
  title: "Source Plan",
  summary: "Use this Plan as Goal input",
  bodyMarkdown: "Implement the source plan.",
  assumptions: [],
  testPlan: ["npm test"],
  sourceSessionId: "session-plan",
  createdAt: "2026-01-01T00:00:00Z",
  updatedAt: "2026-01-01T00:00:01Z",
};

test("Goal keeps an immutable explicit Plan source without sharing Plan lifecycle", () => {
  const setup = fixture();
  const store = setup.create();
  store.start("Execute source", [], sourcePlan);
  sourcePlan.bodyMarkdown = "mutated outside the Goal";
  store.acceptSpec(spec);

  const goal = store.current();
  assert.equal(goal?.stage, "awaiting_approval");
  assert.equal(goal?.sourcePlan?.artifact.bodyMarkdown, "Implement the source plan.");
  assert.equal(goal?.spec?.sourcePlan?.id, "plan-source");
  assert.equal(goal?.spec?.revision, 1);
});

test("switching the foreground session does not detach an active background Goal", () => {
  const setup = fixture();
  const store = setup.create();
  const started = store.start("Keep running");

  const snapshot = store.attach(setup.cwd, "session-2");

  assert.equal(snapshot.goal?.id, started.id);
  assert.equal(snapshot.goal?.sessionId, "session-1");
  assert.match(snapshot.statePath, /session-1\.json$/u);
});

test("legacy Goal plan state migrates to GoalSpec without inventing a Plan reference", () => {
  const setup = fixture();
  const store = setup.create();
  store.start("Legacy Goal");
  writeFileSync(
    store.snapshot().statePath,
    JSON.stringify({
      schemaVersion: 1,
      id: "goal-legacy",
      workspace: setup.cwd,
      sessionId: "session-1",
      objective: "Legacy Goal",
      stage: "awaiting_approval",
      revision: 4,
      constraints: [],
      acceptanceCriteria: ["npm test"],
      plan: {
        title: "Legacy embedded plan",
        bodyMarkdown: "Implement legacy work.",
      },
      planDetails: {
        allowedTools: ["read", "write"],
        allowedPaths: ["src"],
        allowedCommands: ["npm test"],
      },
      tasks: [],
      reviews: [],
      verification: [],
      repairCycles: 0,
      createdAt: "2026-01-01T00:00:00Z",
      updatedAt: "2026-01-01T00:00:01Z",
    }),
  );

  const migrated = setup.create().current();
  assert.equal(migrated?.schemaVersion, 2);
  assert.equal(migrated?.spec?.summary, "Implement legacy work.");
  assert.equal(migrated?.sourcePlan, undefined);
  assert.equal(migrated?.spec?.sourcePlan, undefined);
});

test("goal sidecar restores executing work as paused and marks tasks interrupted", () => {
  const setup = fixture();
  const store = setup.create();
  store.start("Implement the harness");
  store.acceptSpec({
    ...spec,
    allowedPaths: ["agent-host/src"],
    allowedCommands: ["npm test"],
    tasks: [
      {
        id: "host",
        title: "Host",
        description: "Implement host support",
      },
    ],
  });
  store.approveSpec();
  store.updateTask("host", "running");

  const restored = setup.create().current();
  assert.equal(restored?.stage, "paused");
  assert.equal(restored?.previousStage, "executing");
  assert.equal(restored?.tasks[0]?.status, "interrupted");
  assert.match(restored?.lastError ?? "", /restart/u);
});

test("goal restart pauses verification even when no task was running", () => {
  const setup = fixture();
  const store = setup.create();
  store.start("Verify");
  store.acceptSpec(spec);
  store.approveSpec();
  store.transition("verifying");

  const restored = setup.create().current();
  assert.equal(restored?.stage, "paused");
  assert.equal(restored?.previousStage, "verifying");
});

test("goal restart pauses an in-flight background preparation", () => {
  const setup = fixture();
  setup.create().start("Prepare independently");

  const restored = setup.create().current();
  assert.equal(restored?.stage, "paused");
  assert.equal(restored?.previousStage, "preparing");
});

test("allow-for-goal extends only the current persisted lease", () => {
  const setup = fixture();
  const store = setup.create();
  store.start("Implement");
  store.acceptSpec({
    ...spec,
    allowedTools: ["read"],
    allowedPaths: ["src"],
    allowedCommands: [],
  });
  store.approveSpec();
  store.extendLease("bash", { command: "cargo test" });

  const restored = setup.create().current();
  assert.deepEqual(restored?.lease?.allowedTools.sort(), ["bash", "read"]);
  assert.deepEqual(restored?.lease?.allowedCommands, ["cargo test"]);
});

test("workspace goal listing is read-only and sorted by most recent update", () => {
  const setup = fixture();
  const first = setup.create();
  first.start("First goal");
  const second = new GoalStore({
    cwd: setup.cwd,
    sessionId: "session-2",
    rootDir: setup.root,
    createId: () => "goal-2",
    now: () => "2026-02-01T00:00:00Z",
  });
  second.start("Second goal");

  const listing = second.list();
  assert.deepEqual(
    listing.goals.map((goal) => goal.id),
    ["goal-2", "goal-1"],
  );
  assert.equal(second.current()?.id, "goal-2");
});

test("independent review permits one repair cycle and blocks after the second", () => {
  const setup = fixture();
  const store = setup.create();
  store.start("Implement");
  store.acceptSpec(spec);
  store.approveSpec();
  store.transition("verifying");
  store.addVerification({
    result: {
      status: "completed",
      summary: "Tests passed",
      evidence: ["npm test"],
      changedPaths: [],
      verification: [
        {
          command: "npm test",
          exitCode: 0,
          output: "pass",
        },
      ],
      blockers: [],
    },
    agentId: "verifier-1",
  });
  store.transition("reviewing");
  const finding = {
    severity: "high" as const,
    title: "Missing verification",
    evidence: "No test result",
    recommendation: "Run tests",
  };
  const first = store.addReview({
    verdict: "changes_required",
    summary: "Repair required",
    findings: [finding],
    agentId: "reviewer-1",
  });
  assert.equal(first.stage, "executing");
  assert.equal(first.repairCycles, 1);

  store.transition("verifying");
  store.addVerification({
    result: {
      status: "completed",
      summary: "Tests still pass",
      evidence: ["npm test"],
      changedPaths: [],
      verification: [],
      blockers: [],
    },
    agentId: "verifier-2",
  });
  store.transition("reviewing");
  const second = store.addReview({
    verdict: "changes_required",
    summary: "Still failing",
    findings: [finding],
    agentId: "reviewer-2",
  });
  assert.equal(second.stage, "blocked");
  assert.equal(second.repairCycles, 2);
  assert.equal(second.verification.length, 2);
});

test("review repairs rerun only targeted tasks and their dependants", () => {
  const setup = fixture();
  const store = setup.create();
  const completed = {
    status: "completed" as const,
    summary: "done",
    evidence: [],
    changedPaths: [],
    verification: [],
    blockers: [],
  };
  store.start("Targeted repair");
  store.acceptSpec({
    ...spec,
    tasks: [
      { id: "a", title: "A", description: "Change A", allowedPaths: ["a"] },
      { id: "b", title: "B", description: "Change B", allowedPaths: ["b"] },
      {
        id: "c",
        title: "C",
        description: "Build on A",
        dependsOn: ["a"],
        allowedPaths: ["c"],
      },
    ],
  });
  store.approveSpec();
  for (const id of ["a", "b", "c"]) {
    store.updateTask(id, "running");
    store.updateTask(id, "completed", completed);
  }
  store.transition("verifying");
  store.addVerification({ result: completed, agentId: "verifier" });
  store.transition("reviewing");
  const repaired = store.addReview({
    verdict: "changes_required",
    summary: "A needs repair",
    findings: [
      {
        severity: "high",
        title: "A is wrong",
        evidence: "a/file.ts",
        recommendation: "Repair A",
        taskIds: ["a"],
      },
    ],
    agentId: "reviewer",
  });
  assert.equal(repaired.tasks.find((task) => task.id === "a")?.status, "pending");
  assert.equal(repaired.tasks.find((task) => task.id === "b")?.status, "completed");
  assert.equal(repaired.tasks.find((task) => task.id === "c")?.status, "pending");
});

test("Goal and task state machines reject illegal jumps", () => {
  const setup = fixture();
  const store = setup.create();
  store.start("State machine");
  assert.throws(() => store.transition("reviewing"), /Cannot transition/u);
  store.acceptSpec({
    ...spec,
    tasks: [{ id: "task", title: "Task", description: "Do work" }],
  });
  store.approveSpec();
  assert.throws(
    () => store.updateTask("task", "completed"),
    /Cannot transition/u,
  );
});

test("GoalStore rejects invalid task graphs before persisting a spec", () => {
  const store = fixture().create();
  store.start("Validate graph");
  assert.throws(
    () =>
      store.acceptSpec({
        ...spec,
        tasks: [{ id: "empty", title: " ", description: "work" }],
      }),
    /title must not be empty/u,
  );
  assert.throws(
    () =>
      store.acceptSpec({
        ...spec,
        tasks: [
          { id: "same", title: "One", description: "work" },
          { id: "same", title: "Two", description: "work" },
        ],
      }),
    /duplicate task ID/u,
  );
  assert.throws(
    () =>
      store.acceptSpec({
        ...spec,
        tasks: [
          {
            id: "one",
            title: "One",
            description: "work",
            dependsOn: ["missing"],
          },
        ],
      }),
    /unknown task/u,
  );
  assert.throws(
    () =>
      store.acceptSpec({
        ...spec,
        tasks: [
          {
            id: "one",
            title: "One",
            description: "work",
            dependsOn: ["two"],
          },
          {
            id: "two",
            title: "Two",
            description: "work",
            dependsOn: ["one"],
          },
        ],
      }),
    /dependency cycle/u,
  );
  assert.equal(store.current()?.stage, "preparing");
  assert.equal(store.current()?.spec, undefined);
});

test("capability matching is workspace-relative and command-prefix based", () => {
  const setup = fixture();
  assert.equal(
    pathAllowedByLease(setup.cwd, "src/app.ts", ["src"]),
    true,
  );
  assert.equal(
    pathAllowedByLease(setup.cwd, "../secret", ["."]),
    false,
  );
  assert.equal(commandAllowedByLease("cargo test app", ["cargo test"]), true);
  assert.equal(commandAllowedByLease("cargo publish", ["cargo test"]), false);
  assert.equal(commandAllowedByLease("rm -rf target", ["rm"]), false);
  assert.equal(
    commandAllowedByLease("cargo test && touch changed.txt", ["cargo test"]),
    false,
  );
});

test("built-in verifier denies shell composition outside its read-only allowlist", () => {
  const setup = fixture();
  const config = loadHarnessConfig(setup.cwd, {
    homeDir: join(setup.root, "home"),
  });
  const verifier = config.profiles.verifier;
  assert.ok(verifier);
  assert.equal(
    agentPermissionEffect(verifier, "bash", "cargo clippy --all-targets"),
    "allow",
  );
  assert.equal(
    agentPermissionEffect(
      verifier,
      "bash",
      "cargo test && touch changed.txt",
    ),
    "deny",
  );
});

test("trusted project config fully overrides profile fields", () => {
  const setup = fixture();
  const home = join(setup.root, "home");
  const projectDir = join(setup.cwd, ".nabla");
  mkdirSync(projectDir);
  writeFileSync(
    join(projectDir, "config.json"),
    JSON.stringify({
      maxParallel: 99,
      profiles: {
        reviewer: {
          permission: "goal_lease",
          tools: ["read", "write"],
          maxParallel: 99,
        },
      },
    }),
  );

  assert.equal(loadHarnessConfig(setup.cwd, { homeDir: home }).maxParallel, 3);
  saveWorkspaceTrust(setup.cwd, true, { homeDir: home });
  const trusted = loadHarnessConfig(setup.cwd, { homeDir: home });
  assert.equal(trusted.maxParallel, 99);
  assert.equal(
    agentPermissionEffect(trusted.profiles.reviewer!, "write", "src/app.rs"),
    "ask",
  );
  assert.deepEqual(trusted.profiles.reviewer?.tools, ["read", "write"]);
  assert.equal(trusted.profiles.reviewer?.maxParallel, 99);
  assert.match(
    readFileSync(join(home, ".nabla", "config.json"), "utf8"),
    /trustedWorkspaces/u,
  );
});

test("markdown subagents merge globally and from trusted projects", () => {
  const setup = fixture();
  const home = join(setup.root, "home");
  const globalAgents = join(home, ".nabla", "agents");
  const projectAgents = join(setup.cwd, ".nabla", "agents");
  mkdirSync(globalAgents, { recursive: true });
  mkdirSync(projectAgents, { recursive: true });
  writeFileSync(
    join(globalAgents, "docs.md"),
    [
      "---",
      "description: Global docs writer",
      "model: test/global",
      "tools: [read, write]",
      "permission:",
      "  write: ask",
      "maxTurns: 9",
      "---",
      "Write concise documentation.",
    ].join("\n"),
  );
  writeFileSync(
    join(projectAgents, "docs.md"),
    [
      "---",
      "description: Project docs writer",
      "model: test/project",
      "permission:",
      "  write:",
      '    "*": deny',
      '    "docs/**": allow',
      "---",
      "Follow this project's documentation conventions.",
    ].join("\n"),
  );

  const untrusted = loadHarnessConfig(setup.cwd, { homeDir: home });
  assert.equal(untrusted.profiles.docs?.model, "test/global");
  assert.equal(untrusted.profiles.docs?.maxTurns, 9);

  saveWorkspaceTrust(setup.cwd, true, { homeDir: home });
  const trusted = loadHarnessConfig(setup.cwd, { homeDir: home });
  const profile = trusted.profiles.docs!;
  assert.equal(profile.description, "Project docs writer");
  assert.equal(profile.model, "test/project");
  assert.equal(profile.maxTurns, 9);
  assert.deepEqual(profile.tools, ["read", "write"]);
  assert.deepEqual(profile.instructions, [
    "Follow this project's documentation conventions.",
  ]);
  assert.equal(
    agentPermissionEffect(profile, "write", "docs/guide.md"),
    "allow",
  );
  assert.equal(
    agentPermissionEffect(profile, "write", "src/app.ts"),
    "deny",
  );
});

test("custom profiles use a safe baseline and ordered permission rules", () => {
  const setup = fixture();
  const home = join(setup.root, "home");
  const agents = join(home, ".nabla", "agents");
  mkdirSync(agents, { recursive: true });
  writeFileSync(
    join(agents, "audit.md"),
    [
      "---",
      "description: Audit changes",
      "tools: [read, bash]",
      "permission:",
      "  bash:",
      '    "*": deny',
      '    "cargo test*": allow',
      "isolation:",
      "  mode: worktree",
      "  integration: ask",
      "---",
      "Audit and verify without editing files.",
    ].join("\n"),
  );
  writeFileSync(
    join(agents, "writer.md"),
    "---\ndescription: Write files\ntools: [read, write]\n---\nWrite the requested file.",
  );

  const config = loadHarnessConfig(setup.cwd, {
    homeDir: home,
  });
  const profile = config.profiles.audit!;
  assert.equal(agentPermissionEffect(profile, "read", "src/lib.rs"), "allow");
  assert.equal(agentPermissionEffect(profile, "bash", "cargo test app"), "allow");
  assert.equal(agentPermissionEffect(profile, "bash", "cargo publish"), "deny");
  assert.equal(profile.maxParallel, 1);
  assert.equal(profile.maxTurns, 24);
  assert.deepEqual(profile.isolation, {
    mode: "worktree",
    integration: "ask",
  });
  assert.deepEqual(config.profiles.writer?.isolation, {
    mode: "none",
    integration: "source",
  });
  assert.deepEqual(config.profiles.worker?.isolation, {
    mode: "auto",
    integration: "source",
  });
  assert.equal(
    agentPermissionEffect(config.profiles.writer!, "write", "src/app.ts"),
    "ask",
  );
});

test("trust updates preserve unrelated global config fields", () => {
  const setup = fixture();
  const home = join(setup.root, "home");
  const configDir = join(home, ".nabla");
  mkdirSync(configDir, { recursive: true });
  writeFileSync(
    join(configDir, "config.json"),
    JSON.stringify({
      schemaVersion: 1,
      maxParallel: 7,
      customFutureField: { keep: true },
      profiles: {
        worker: { model: "test/model" },
      },
    }),
  );

  saveWorkspaceTrust(setup.cwd, true, { homeDir: home });
  const raw = JSON.parse(
    readFileSync(join(configDir, "config.json"), "utf8"),
  );
  assert.deepEqual(raw.customFutureField, { keep: true });
  assert.equal(raw.profiles.worker.model, "test/model");
  assert.equal(raw.maxParallel, 7);
});

test("invalid markdown disables only that definition and reports diagnostics", () => {
  const setup = fixture();
  const home = join(setup.root, "home");
  const agents = join(home, ".nabla", "agents");
  mkdirSync(agents, { recursive: true });
  writeFileSync(
    join(agents, "broken.md"),
    "---\ndescription: Missing prompt\n---\n",
  );
  writeFileSync(
    join(agents, "valid.md"),
    "---\ndescription: Valid\n---\nRead the repository.",
  );
  writeFileSync(
    join(agents, "invalid-field.md"),
    "---\ndescription: Invalid field\nmaxTurns: 0\n---\nRead the repository.",
  );

  const config = loadHarnessConfig(setup.cwd, { homeDir: home });
  assert.equal(config.profiles.broken, undefined);
  assert.equal(config.profiles.valid?.description, "Valid");
  assert.equal(config.profiles["invalid-field"]?.disabled, true);
  assert.match(config.diagnostics[0]?.message ?? "", /non-empty prompt/u);
});

test("untrusted resources keep global context and exclude project instructions", () => {
  const files = [
    { path: "/agent/AGENTS.md", content: "global" },
    { path: "/workspace/AGENTS.md", content: "project" },
  ];
  assert.deepEqual(
    filterContextFilesByTrust(files, "/agent", false),
    [files[0]],
  );
  assert.deepEqual(filterContextFilesByTrust(files, "/agent", true), files);
});
