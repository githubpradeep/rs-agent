# Agent evals (real-world tasks)

Runnable end-to-end checks against a live provider — not unit tests.

Each task is a small workspace + prompt + `check.sh` that asserts filesystem /
command outcomes. The harness copies the task into a temp dir, runs:

```text
rs-agent -a --timeout … -p "$(cat prompt.txt)"
```

then executes `check.sh`.

## Quick start

```bash
# Needs a release binary + API credentials for the provider you choose
cargo build --release

# Default: anthropic (or whatever is in ~/.rs-agent/config.toml if you omit --provider)
./scripts/eval-agent.sh

# Explicit provider/model
./scripts/eval-agent.sh --provider anthropic --model claude-sonnet-4-20250514
./scripts/eval-agent.sh --provider amazon-bedrock --model us.anthropic.claude-opus-4-6-v1

# Subset
./scripts/eval-agent.sh --only write_hello,fix_bug
./scripts/eval-agent.sh --list
```

Exit code is non-zero if any task fails.

## Tasks

| Id | What it measures |
|----|------------------|
| `write_hello` | Create a file with exact contents (`write` tool) |
| `fix_bug` | Read + edit a tiny Python bug, then run it |
| `find_secret` | Explore a small tree and recover a hidden token |
| `rlm_needle` | RLM USP: load padded corpus in `repl`, find token, summarize slice |
| `rlm_escalate` | Huge corpus (~90KB); agent should follow `[rlm_escalate]` / use `repl` |
| `edit_whitespace_drift` | Soft edit apply when file has trailing whitespace drift |
| `post_edit_type_error` | Fix after run/diagnostics feedback; don't stop on broken assert |
| `bash_long_output` | Recover needle from spilled truncated bash output |
| `parallel_same_file` | Multiple edits to one file leave a valid result |

Optional / slower (opt-in):

| Id | Flag |
|----|------|
| `webfetch_title` | `--with-network` — fetch a public URL and mention a known phrase |

## Notes

- These evals cover **everyday coding** (write/edit/search) plus **RLM** tasks.
- Use a **capable** model. Free/tiny routers often fail tool schemas.
- `-a` auto-approves tools (evals are isolated in `/tmp`).
- Cost: a few cents to a couple of dollars per full run depending on model.
- CI job `rlm-demo` runs `demo-rlm.sh` + `rlm_escalate` when `ANTHROPIC_API_KEY` /
  `OPENAI_API_KEY` secrets are set (skips otherwise; `continue-on-error`).

See also: [`docs/demo.md`](../docs/demo.md) for the public Deep Context demo
(`demo-deep-context.sh`). `demo-rlm.sh` remains the scripted CI harness.
