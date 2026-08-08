# Nabla Repository Guidelines

## Module Map

Nabla has a Rust core (`src/`) that renders the TUI and a TypeScript host
(`agent-host/src/`) that runs inside Pi and owns sessions, context, plans,
permissions, and subagent isolation. Pi remains the source of truth for
sessions and compaction.

Rust core (`src/`, entry `main.rs`):

- `app.rs` + `app/` — reducer facade, split by input/event source.
- `state.rs` + `state/` — state facade and per-domain state.
- `runtime.rs` — `EffectDispatcher` executes reducer effects.
- `pi/` + `process/` — Pi JSONL client/events and process spawn/guard.
- `host/` + `rpc.rs` — `HostClient` RPCs to the TS host and the shared
  `JsonLineRpcPeer`.
- `ui/`, `sandbox/`, `file_references/`, `selection.rs`, `browser.rs`,
  `command.rs`, `config.rs`, `event.rs` — TUI, sandbox profiles, file
  references, selection helpers, URL safety, command routing, config, events.

TypeScript host (`agent-host/src/`, entry `main.ts`):

- `app/` — composition root and lifecycle (`create-host-app.ts`,
  `host-app.ts`).
- `protocol/` — command router + `commands/*`, `schemas/*`, `contracts.ts`,
  shared `validation.ts`, event publisher.
- `features/` — domain services: permissions, sessions, context, plans,
  subagents, workspace, auth, bootstrap, models, interactions.
- `runtime/`, `transport/`, `persistence/`, `diagnostics/` — Pi integration,
  control socket, atomic JSON, diagnostics.

Cross-language boundaries:

- **Rust → TS host**: Unix socket JSONL (`NABLA_CONTROL_SOCKET`); wire shapes
  defined in TS `protocol/schemas/*` and Rust `src/host/dto.rs`, validated by
  `protocol-fixtures/` golden files on both sides.
- **TS host → Pi**: `@earendil-works/pi-coding-agent` runtime; Pi owns
  sessions and compaction.
- **Rust → Pi**: `PiClient`/`PiEventReceiver`; `src/process/` owns spawn.

The repo root has a `.codegraph` index; run `codegraph explore "<symbol or
question>"` before grep/find to find existing implementations and callers.

## Existing Shared Tools & Helpers

Check these before writing a new helper. Add new shared code only to the
listed canonical modules.

### TypeScript

- `protocol/validation.ts` — `isJsonObject`, `stringArray`, `stringField`,
  `optionalStringField`, `stringArrayField`, `enumField`, `validAgentName`,
  `errorMessage`, `isStringRecord`, `sanitizeLine`, `stringValue`.
- `permissions/shell/digest.ts` — `canonicalJson`, `digestValue`, `sha256Hex`,
  `npmScriptDigest`.
- `permissions/filesystem/path.ts` — `isPathWithin`, `workspaceRelativePath`,
  `canonicalPath`, `canonicalizePath`, `normalizePath`,
  `normalizeToolInputPaths`, `assertWorkspaceRelativePath`, `patternMatches`.
- `permissions/workspace-identity.ts` — `resolveWorkspaceIdentity`,
  `workspaceInvalidationKeys`, `fileDigest`, `invalidationKeysValid`.
- `permissions/shell/rules.ts` — `READ_ONLY_TOOL_NAMES`, `MUTATING_TOOL_NAMES`,
  `THINKING_LEVELS`, read-only/high-risk command classifiers.
- `permissions/policy/builtin.ts` — `buildCredentialDenyRules`,
  `buildReadOnlyBashRules`, `buildSandboxBashRules`.
- `permissions/execution/sandbox-config.ts` — `SandboxConfig`,
  `EMPTY_SANDBOX_CONFIG` (global `~/.nabla/config.json` `sandbox` section:
  `writableRoots`, `unixSockets.allow/deny`; project config cannot expand).
- `runtime/path-utils.ts` — `expandHomePath`.
- `persistence/atomic-json.ts` — `writeAtomicFile`, `writeAtomicJson`,
  `writeAtomicJsonSync`.
- `protocol/pending-request-registry.ts` — `PendingRequestRegistry`;
  `protocol/command-lanes.ts` — `CommandLanes`.
- `protocol/message-content.ts` — `messageContentText`, `displayMessageText`,
  `parseFileReferenceEnvelope`, `compactionFileDetails`;
  `protocol/subagent-output.ts` — `parseSubagentOutput`.
- `features/context/estimator.ts` — token estimation, `messageRole`,
  `normalizeToolName`, `normalizeArguments`, `firstString`, `safeSummary`,
  `collectToolCalls`.
- `tool-diff.ts` — `newFileDisplayDiff`.

### Rust

- `src/rpc.rs` — `JsonLineRpcPeer`, `RpcRequest`/`RpcResponse`,
  `encode_line`/`parse_incoming_line`.
- `src/selection.rs` — `previous_wrapped`, `next_wrapped`, `page_backward`,
  `page_forward`, `centered_visible_start`.
- `src/browser.rs` — `is_safe_web_url`; `src/command.rs` — `CommandCatalog`
  (route/completion/suggestion).
- `src/file_references/matcher.rs` — `match_score`, `is_subsequence`,
  `path_depth`, `slash_path`.
- `src/process/` — `PiProcessConfig`, `PiChildGuard`;
  `src/sandbox/profile.rs` — `compile`.
- `src/host/` — `HostClient` named RPC methods and `dto.rs` wire types.

## Avoid Duplicating Development

- Search the shared list above and run `codegraph explore` before writing a
  new helper; never copy a helper into a feature directory.
- Do not re-implement: JSON validation/decoding (`validation.ts`), sha256
  digests (`digest.ts`), path canonicalization and workspace-relative
  conversion (`path.ts`), agent-name validation and error/string formatting
  (`validation.ts`), npm script digests (`digest.ts`), workspace trust checks
  (`config.ts` `workspaceIsTrusted`), or JSONL framing (`rpc.rs`).
- Keep paired cross-language implementations in sync: fuzzy matching
  (`catalog.ts` ↔ `matcher.rs`) and tool path key lists (`tool_diff.rs` ↔
  `estimator.ts` `firstString`). Protocol changes must update both wire
  definitions and the `protocol-fixtures/` golden files.
- Reuse `PendingRequestRegistry` for request queues (approval and question
  flows already use it); reuse permissions kernel/evaluator, sessions, and
  context services instead of building parallel state or validation.
- TS and Rust each keep their own protocol validation by design; within each
  language there must be exactly one shared implementation.

## Build, Test, and Development Commands

Run from the repository root unless noted:

```sh
cargo check                            # fast Rust compile validation
cargo test                             # run Rust tests
cargo fmt --all -- --check             # verify Rust formatting
cargo clippy --all-targets -- -D warnings
cd agent-host && npm run typecheck     # strict TypeScript checking
cd agent-host && npm test              # Node test runner (scripts/test.mjs redirects
                                       # PI_CODING_AGENT_DIR/PI_CODING_AGENT_SESSION_DIR to tmpdir)
cd agent-host && npm run host          # run the host directly
```

Unix socket / control-server tests require the app sandbox to allow local IPC:
run them through `./target/debug/nabla __sandbox-exec` with `readWrite`
including the workspace and `tmpdir()` when the calling shell denies network.

Use the Node version declared in `agent-host/package.json`.

## Coding Style & Naming Conventions

Format Rust with `rustfmt`; use `snake_case` for modules/functions and
`PascalCase` for types. TypeScript uses strict checking, two-space
indentation, `camelCase` values/functions, and `PascalCase` types. Keep
protocol JSON fields `camelCase`. Prefer the shared helpers listed in
"Existing Shared Tools & Helpers" over adding parallel implementations.

## Testing Guidelines

Add focused regression tests with every behavior change. Stateful features
need success, cancellation, failure, concurrency, and recovery coverage where
applicable. Cross-language protocol changes must update and validate golden
fixtures on both sides. Run both complete test suites plus formatting,
Clippy, and TypeScript type checking before review.

## Architecture Constraints & Safety

Keep Plan as its own workflow. Pi remains the source of truth for sessions
and compaction. Preserve user work and tool output; do not trim Git-reported
paths or bypass workspace-boundary helpers. Never log credentials or full
authentication input.

## Commit & Pull Request Guidelines

Use short imperative subjects, optionally scoped, for example
`worktree: make integration idempotent`. Pull requests should describe
behavior and state-transition risks, list verification commands, link
relevant issues, and call out protocol or fixture updates.
