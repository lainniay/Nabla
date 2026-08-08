import assert from "node:assert/strict";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import test from "node:test";

import type { PermissionIntent } from "../model.ts";
import { buildSandboxProfile } from "./sandbox-profile.ts";
import type { SandboxCapability } from "./sandbox-capability.ts";

const enforced: SandboxCapability = {
  mode: "enforced",
  backend: "seatbelt",
  supportsFilesystemIsolation: true,
  supportsNetworkIsolation: true,
};

function intent(atoms: PermissionIntent["atoms"]): PermissionIntent {
  return {
    id: "intent-1",
    toolCallId: "t1",
    sessionId: "session-1",
    workspaceId: "workspace-1",
    tool: "bash",
    normalizedInput: { command: "test" },
    atoms,
    digest: "digest",
  };
}

test("enforced profile grants approved writes and keeps credentials denied", () => {
  const profile = buildSandboxProfile(
    intent([
      {
        kind: "file",
        operation: "write",
        path: "/workspace/src/lib.rs",
      },
      {
        kind: "network",
        operation: "connect",
        host: "github.com",
      },
    ]),
    "/workspace",
    enforced,
  );
  assert.equal(profile.mode, "enforced");
  assert.equal(profile.backend, "native");
  assert.ok(profile.filesystem.readWrite.includes(resolve("/workspace/src/lib.rs")));
  assert.ok(profile.filesystem.denyRead.some((path) => path.endsWith(".ssh")));
  assert.equal(profile.network, "allowed");
});

test("degraded capability yields an unsandboxed profile", () => {
  const profile = buildSandboxProfile(intent([]), "/workspace", {
    mode: "degraded",
    backend: "none",
    supportsFilesystemIsolation: false,
    supportsNetworkIsolation: false,
  });
  assert.equal(profile.mode, "degraded");
  assert.equal(profile.backend, "none");
  assert.equal(profile.network, "blocked");
});

test("workspace and tmpdir are always writable", () => {
  const profile = buildSandboxProfile(intent([]), "/workspace", enforced);
  assert.ok(profile.filesystem.readWrite.includes("/workspace"));
  assert.ok(profile.filesystem.readWrite.includes(tmpdir()));
  assert.deepEqual(profile.unixSockets, { allow: [], deny: [] });
});

test("configured writable roots and unix sockets flow into the profile", () => {
  const profile = buildSandboxProfile(intent([]), "/workspace", enforced, {
    writableRoots: ["/srv/nabla-cache"],
    unixSockets: {
      allow: ["/var/run/nabla.sock"],
      deny: ["/workspace/.env.sock"],
    },
  });
  assert.ok(profile.filesystem.readWrite.includes(resolve("/srv/nabla-cache")));
  assert.deepEqual(profile.unixSockets, {
    allow: [resolve("/var/run/nabla.sock")],
    deny: [resolve("/workspace/.env.sock")],
  });
});
