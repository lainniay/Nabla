# Configurable subagents

Nabla loads subagent profiles from these sources, with later sources
overriding earlier fields:

1. Built-in `planner`, `worker`, `verifier`, and `reviewer` profiles.
2. `~/.nabla/config.json`.
3. `~/.nabla/agents/*.md`.
4. A trusted workspace's `.nabla/config.json`.
5. A trusted workspace's `.nabla/agents/*.md`.

Project files are ignored until `/trust on` is accepted. Use `/agents reload`
or `/reload` after editing a profile.

```markdown
---
description: Reviews Rust changes
model: openai/gpt-5
thinkingLevel: high
skills: []
tools: [read, grep, find, ls, bash]
maxParallel: 2
maxTurns: 16
isolation:
  mode: auto
  integration: source
disabled: false
permission:
  read: allow
  bash:
    "*": ask
    "cargo test*": allow
    "cargo clippy*": allow
---
Review correctness, regressions, and missing tests. Return concise,
artifact-backed evidence.
```

The Markdown filename is the profile name. New Markdown profiles require a
non-empty `description` and body. If `model` or `thinkingLevel` is omitted,
the subagent inherits the values from the session that started it.

`tools` controls which tools exist in the subagent session. `permission`
controls each exposed tool with `allow`, `ask`, or `deny`. A permission may be
a single effect or an ordered map of resource glob to effect; the last matching
entry wins. Paths are workspace-relative and bash resources are normalized
commands. Reads default to `allow`; edit, write, and bash default to `ask`.

`isolation.mode` accepts `none`, `auto`, or `worktree`. The built-in worker
uses `auto`: Nabla creates a detached worktree before the model starts, or
falls back to one serialized writer when the workspace is not a Git
repository. Explicit `worktree` mode fails outside Git instead of silently
losing isolation. The host owns worktree creation, patch capture, integration,
and cleanup; subagents are not asked to run `git worktree` themselves.

`isolation.integration` accepts `source`, `auto`, `ask`, or `manual`. `source`
automatically applies clean Goal patches and asks before applying direct
`/agent` results. Conflicts remain outside the main workspace and may be sent
to one isolated resolver attempt. If that attempt fails, the host returns to
the integration prompt and will not start a second resolver for the same
patch.

Goal leases remain an upper bound. Credential paths, paths outside the
workspace, and high-risk commands always require confirmation even when a
profile says `allow`. Subagents started while the foreground session is in Plan
mode are read-only.

Commands:

- `/agent` opens the profile selector.
- `/agent <name> <task>` starts a background subagent.
- `/agents` shows profiles, diagnostics, and active runs.
- `/agents reload` reloads configuration.
- `/agents cancel <agent-id>` cancels a queued or running subagent.
- `/agents apply <agent-id>` applies a pending clean patch.
- `/agents resolve <agent-id>` starts one isolated conflict resolver.
- `/agents keep <agent-id>` keeps the managed worktree and patch.
- `/agents discard <agent-id>` removes the managed worktree and patch.

Direct subagent results are shown in the transcript and stored as a hidden,
model-visible message in the session that started them. They do not
automatically trigger another main-model request.
