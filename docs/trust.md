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
| `~/.rs-agent/` | config, sessions, secrets, trust/permissions |
| `.rs-agent/` in the repo | hooks |
| `python3` | Deep Context `repl` (see below) |

There is **no product telemetry**. The process talks to the LLM provider you
configured and to URLs the agent fetches.

## Permission modes

| Mode | Behavior |
|------|----------|
| Default TUI | Risky tools prompt: once / path-scoped / always-trust-this-project / deny. Edit prompts can show a diff. |
| `--auto-mode` | Read-only tools auto-approve (`read`, `grep`, `ls`, `find`, `webfetch`, `websearch`). Writes and `bash` still prompt. |
| `-a` / `approve = true` (**YOLO**) | Every tool call runs without a prompt. |

Dangerous-`bash` heuristics (`rm -rf /`, `mkfs`, curl-pipe-to-shell, …) raise a
louder prompt in the TUI. They are **substring guards, not a sandbox**. YOLO
skips the prompt.

API keys live in the environment or `~/.rs-agent/secrets.toml`. Do not commit
that file. Do not paste keys into chat.

## Deep Context `repl`

The `repl` tool is a persistent **`python3` process** with a host-mediated
`llm_query` / `agent_query`. It is **not isolated**:

- `exec` of model-written Python
- full builtins, `os`, and filesystem access (same user as rs-agent)
- no Docker / jail by default (optional isolation is later work)

Huge `read`s may auto-escalate into `repl` (`[rlm_escalate]`). If you do not
want that, do not run Deep Context on untrusted trees, or run without `python3`
on `PATH` (the tool will error instead).

## What this is not

- Not a multi-tenant cloud agent
- Not SOC2 / SSO / audit-log product
- Not an OS sandbox (macOS seatbelt, seccomp, or containers)
- Not a substitute for `git` — commit and branch as you would with any coding agent

## If something goes wrong

- TUI: `Esc` aborts the current turn (including running `bash`)
- Trust reset: delete `~/.rs-agent/trust.json` / `permissions.json` or `/trust`
- Repo: `git status` / `git checkout`
