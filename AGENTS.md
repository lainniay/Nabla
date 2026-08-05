# Repository Guidelines

## Project Structure & Module Organization

Nabla has a Rust core library and a cooperating TypeScript host:

- `src/app.rs` is the application reducer facade; `src/app/` contains submodules split by input, workflow, and protocol event source.
- `src/state.rs` is the state facade; `src/state/` groups sessions, context, resources, agents, planning, authentication, navigation, and transcript concerns.
- `host.rs`, `pi_process.rs`, and `rpc.rs` handle JSONL communication with Pi and the TypeScript host.
- `agent-host/src/` contains the TypeScript Pi host. Domain logic lives in files such as `context-manager.ts`, `harness.ts`, and `worktree.ts`; shared validation, policy, and persistence utilities live under `protocol/`, `policy/`, and `persistence/`.
- TypeScript tests are colocated as `*.test.ts`. Rust unit tests are generally colocated in their source modules.
- `protocol-fixtures/` holds cross-language golden fixtures. `SUBAGENTS.md` documents subagent configuration and behavior.

## Build, Test, and Development Commands

Run from the repository root unless noted:

```sh
cargo check                            # fast Rust compile validation
cargo test                             # run Rust tests
cargo fmt --all -- --check             # verify Rust formatting
cargo clippy --all-targets -- -D warnings
cd agent-host && npm run typecheck     # strict TypeScript checking
cd agent-host && npm test              # Node test runner
cd agent-host && npm run host          # run the host directly
```

Use the Node version declared in `agent-host/package.json`.

## Coding Style & Naming Conventions

Format Rust with `rustfmt`; use `snake_case` for modules/functions and `PascalCase` for types. TypeScript uses strict checking, two-space indentation, `camelCase` values/functions, and `PascalCase` types. Keep protocol JSON fields `camelCase`. Prefer existing shared policy, validation, persistence, selection, and RPC helpers over adding parallel implementations.

## Testing Guidelines

Add focused regression tests with every behavior change. Stateful features need success, cancellation, failure, concurrency, and recovery coverage where applicable. Cross-language protocol changes must update and validate golden fixtures on both sides. Run both complete test suites plus formatting, Clippy, and TypeScript type checking before review.

## Architecture & Safety

Keep Plan as its own workflow. Pi remains the source of truth for sessions and compaction. Preserve user work and tool output; do not trim Git-reported paths or bypass workspace-boundary helpers. Never log credentials or full authentication input.

## Commit & Pull Request Guidelines

This repository currently has no Git history, so no established convention can be inferred. Use short imperative subjects, optionally scoped, for example `worktree: make integration idempotent`. Pull requests should describe behavior and state-transition risks, list verification commands, link relevant issues, and call out protocol or fixture updates.
