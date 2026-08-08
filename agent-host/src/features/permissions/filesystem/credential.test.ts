import assert from "node:assert/strict";
import test from "node:test";

import { isCredentialPath } from "./credential.ts";

test("credential path markers match with case and separator normalization", () => {
  for (const path of [
    "/Users/me/.ssh/config",
    "C:\\Users\\me\\.aws\\credentials",
    "/workspace/.env",
    "/var/keys/auth.json",
    "/Users/me/.config/gcloud/application_default_credentials.json",
    "/srv/credentials.json",
    "/Users/ME/.SSH/config",
  ]) {
    assert.equal(isCredentialPath(path), true, path);
  }
});

test("ordinary workspace paths are not credential paths", () => {
  for (const path of [
    "/workspace/src/lib.ts",
    "/workspace/envelopes",
    "/Users/me/.ssh-notes",
    "/workspace/.aws-notes/readme.md",
    "/workspace/auth-json.ts",
  ]) {
    assert.equal(isCredentialPath(path), false, path);
  }
});
