# Trust model

rs-agent runs tools on **your machine**, in **your project directory**, with
**your credentials**. Treat it like handing a skilled intern your shell — not
like a hosted sandbox.

This page is the contract. Permissions in the TUI are a prompt UX, not isolation.

## What the binary may touch

| Surface | What happens |
|---------|----------------|
| Project cwd | `read` / `edit` / `write` / `apply_patch` / `bash` / `grep` / `find` / `ls` |
| Network | `websearch`, `webfetch`, provider HTTP APIs |
| `~/.rs-agent/` | config, sessions, secrets, trust/permissions, seats |
| `.rs-agent/` in the repo | beads, fleet status/logs, mail, hooks |
| `python3` | Deep Context `repl` (see below) |

There is **no product telemetry**. The process talks to the LLM provider you
configured and to URLs the agent fetches.

## Permission modes

| Mode | Behavior |
|------|----------|
| Default TUI | Risky tools prompt: once / path-scoped / always-trust-this-project / deny. Edit prompts can show a diff. |
| `--auto-mode` | Read-only tools auto-approve (`read`, `grep`, `ls`, `find`, `webfetch`, `websearch`). Writes and `bash` still prompt. |
| `-a` / `approve = true` (**YOLO**) | Every tool call runs without a prompt. |
| `rs-agent worker` | Always YOLO. Unattended factory cannot sit on a permission card. |

Dangerous-`bash` heuristics (`rm -rf /`, `mkfs`, curl-pipe-to-shell, …) raise a
louder prompt in the TUI. They are **substring guards, not a sandbox**. YOLO
and workers skip the prompt.

API keys live in the environment or `~/.rs-agent/secrets.toml`. Do not commit
that file. Do not paste keys into beads, wishes, or chat.

## Deep Context `repl`

The `repl` tool is a persistent **`python3` process** with a host-mediated
`llm_query` / `agent_query`. It is **not isolated**:

- `exec` of model-written Python
- full builtins, `os`, and filesystem access (same user as rs-agent)
- no Docker / jail by default (optional isolation is later work)

Huge `read`s may auto-escalate into `repl` (`[rlm_escalate]`). If you do not
want that, do not run Deep Context on untrusted trees, or run without `python3`
on `PATH` (the tool will error instead).

## Overnight factory

`worker --loop` and `fleet up` claim beads and implement until budget or an
empty ready queue. That means:

1. **YOLO** — tools execute without you.
2. **Per-seat git worktree by default** — `fleet up` checks out
   `.rs-agent/worktrees/<seat>` so two Fleet seats do not edit the same files.
   Beads and fleet logs stay shared. `--shared-worktree` opts out (seats can
   overwrite each other).
3. **Crash recovery** — stream drops retry; stale leases expire and can be
   reclaimed. A killed worker does not freeze the graph forever, but in-flight
   file edits may be partial if a process is `kill -9` mid-write (edits use
   temp+rename when they finish).

Use overnight only on repos you are willing to reset. Review closed beads in
the morning before you land.

## What this is not

- Not a multi-tenant cloud agent
- Not SOC2 / SSO / audit-log product
- Not an OS sandbox (macOS seatbelt, seccomp, or containers)
- Not a substitute for `git` — commit and branch as you would with any coding agent

## If something goes wrong

- TUI: `Esc` aborts the current turn (including running `bash`)
- Fleet: `rs-agent fleet down` then inspect `.rs-agent/fleet/<seat>.log`
- Trust reset: delete `~/.rs-agent/trust.json` / `permissions.json` or `/trust`
- Repo: `git status` / `git checkout` / worktree remove
