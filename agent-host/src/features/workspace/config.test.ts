import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { loadHarnessConfig } from "./config.ts";
import {
  filterContextFilesByTrust,
  saveWorkspaceTrust,
} from "./trust.ts";
import type { AgentProfile } from "../subagents/profile-model.ts";
import {
  compileAgentProfileRules,
  profileToolEffect,
} from "../permissions/policy/profile-compiler.ts";

function fixture() {
  const root = mkdtempSync(join(tmpdir(), "nabla-test-"));
  const cwd = join(root, "workspace");
  mkdirSync(cwd);
  return {
    root,
    cwd,
  };
}

test("profile rules match workspace-relative paths", () => {
  const setup = fixture();
  const profile: AgentProfile = {
    description: "Writer",
    source: "builtin",
    skills: [],
    tools: ["read", "write"],
    permission: {
      write: [{ resource: "src/**", effect: "allow" }],
    },
    maxParallel: 1,
    maxTurns: 10,
    isolation: { mode: "none", integration: "source" },
    disabled: false,
    instructions: [],
  };
  assert.equal(
    profileToolEffect(profile, "write", "src/app.ts", setup.cwd),
    "allow",
  );
  assert.equal(
    profileToolEffect(profile, "write", "../secret", setup.cwd),
    "ask",
  );
});

test("built-in verifier does not auto-allow shell commands", () => {
  const setup = fixture();
  const config = loadHarnessConfig(setup.cwd, {
    homeDir: join(setup.root, "home"),
  });
  const verifier = config.profiles.verifier;
  assert.ok(verifier);
  assert.equal(
    profileToolEffect(
      verifier,
      "bash",
      "cargo clippy --all-targets",
      setup.cwd,
    ),
    "deny",
  );
  assert.equal(
    profileToolEffect(
      verifier,
      "bash",
      "cargo test && touch changed.txt",
      setup.cwd,
    ),
    "deny",
  );
});

test("trusted project config cannot expand profile permissions", () => {
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
          permission: { write: "allow" },
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
    profileToolEffect(
      trusted.profiles.reviewer!,
      "write",
      "src/app.rs",
      setup.cwd,
    ),
    "deny",
  );
  assert.deepEqual(trusted.profiles.reviewer?.tools, ["read"]);
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
    profileToolEffect(profile, "write", "docs/guide.md", setup.cwd),
    "deny",
  );
  assert.equal(
    profileToolEffect(profile, "write", "src/app.ts", setup.cwd),
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
  assert.equal(
    profileToolEffect(profile, "read", "src/lib.rs", setup.cwd),
    "allow",
  );
  assert.equal(
    profileToolEffect(profile, "bash", "cargo test app", setup.cwd),
    "deny",
  );
  assert.equal(
    profileToolEffect(profile, "bash", "cargo publish", setup.cwd),
    "deny",
  );
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
    profileToolEffect(
      config.profiles.writer!,
      "write",
      "src/app.ts",
      setup.cwd,
    ),
    "ask",
  );
});

test("profile permissions compile to kernel rules", () => {
  const setup = fixture();
  const profile: AgentProfile = {
    description: "Audit",
    source: "builtin",
    skills: [],
    tools: ["read", "bash"],
    permission: {
      bash: [
        { resource: "*", effect: "deny" },
        { resource: "cargo test*", effect: "allow" },
      ],
      read: [{ resource: "src/**", effect: "ask" }],
    },
    maxParallel: 1,
    maxTurns: 10,
    isolation: { mode: "none", integration: "source" },
    disabled: false,
    instructions: [],
  };
  const rules = compileAgentProfileRules(profile, setup.cwd);
  assert.ok(
    rules.some((rule) =>
      rule.effect === "deny" &&
      rule.matcher.kind === "tool" &&
      rule.matcher.tool === "bash"),
  );
  assert.ok(
    rules.some((rule) =>
      rule.effect === "allow" &&
      rule.matcher.kind === "shell_command" &&
      rule.matcher.pattern === "cargo test*"),
  );
  assert.ok(
    rules.some((rule) =>
      rule.effect === "ask" &&
      rule.matcher.kind === "file" &&
      rule.matcher.operation === "read" &&
      rule.matcher.pattern === true),
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
