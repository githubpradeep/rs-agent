# Contributing to rs-agent

Thanks for taking a look. rs-agent is a local overnight coding factory (Rust CLI/TUI,
beads/fleet, skills) with a Deep Context core (persistent Python REPL + `llm_query` /
`agent_query`) so large context stays out of the model window. Product sequence:
[`docs/productize.md`](docs/productize.md). Trust model: [`docs/trust.md`](docs/trust.md).

## Getting set up

Requirements:

- Rust (stable toolchain — see `rustup show` / install via [rustup.rs](https://rustup.rs))
- `python3` on `PATH` (used by the Deep Context `repl` tool)
- An API key for at least one provider (Anthropic recommended — see the README)

Clone and build:

```sh
git clone https://github.com/githubpradeep/rs-agent.git
cd rs-agent
cargo build --release
```

Run it:

```sh
export ANTHROPIC_API_KEY=sk-...
cargo run --release -- --provider anthropic
```

## Before you open a PR

Run these locally; CI runs the same checks on every push and PR:

```sh
cargo fmt --all              # format
cargo fmt --all -- --check   # verify formatting (what CI runs)
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

All four must be clean. Please don't submit a PR with `#[allow(...)]` added just to silence
clippy unless the lint is a genuine false positive — explain why in the PR description if so.

## PR expectations

- Keep PRs focused — one logical change per PR is much easier to review than a grab-bag.
- Update docs (`README.md`, `docs/*.md`, `--help` text) when behavior or flags change.
- Add/extend tests for new tools, providers, or RLM behavior where practical.
- Prefer small, incremental changes to the agent loop and RLM core — they're load-bearing for
  every other feature, so regressions there are expensive.
- Describe *why* in the PR body, not just *what* — especially for anything touching permissions,
  abort/steer, or provider request shaping.
- If you're adding a new provider, follow the existing `Provider` trait implementations in
  `src/ai/` (anthropic, openai, bedrock, opencode) as a template.

## Project layout (orientation)

```
src/
  agent/     agent loop, state, tool dispatch, abort/steer control
  ai/        provider trait + Anthropic/OpenAI/Bedrock/OpenCode implementations
  cli/       clap argument definitions
  context/   AGENTS.md/CLAUDE.md discovery, project commands, system prompt assembly
  permission/ trust store + permission prompts for risky tools
  rlm/       RLM REPL session, call tree, host glue
  session/   session persistence (~/.rs-agent/sessions)
  tools/     read/write/edit/bash/grep/ls/find/websearch/webfetch/repl
  tui/       ratatui-based terminal UI
prompts/     default system prompt (prompts/system.md)
docs/        skills.md, keymap.md reference docs
example/     sample scripts/docs used to exercise the RLM workflow
reference/   local research notes only — not part of the published crate, see below
```

## Skills

Skills are markdown files with frontmatter that teach the agent a workflow. They're discovered
from (in this order) a project-local `.rs-agent/skills/` directory and the user-global
`~/.rs-agent/skills/` directory. See [`docs/skills.md`](docs/skills.md) for the format and
[`docs/keymap.md`](docs/keymap.md) for how skills surface in the TUI (`/skills`, `/skill <name>`).

If you're contributing a new skill, drop it in `skills/` at the repo root (a repo-local skill
pack shipped alongside rs-agent) and follow the frontmatter conventions in `docs/skills.md`.

## Configuration

Runtime config lives under `~/.rs-agent/`:

- `~/.rs-agent/config.toml` — default provider/model/approve/depth settings (see README for the
  current format; this is actively evolving)
- `~/.rs-agent/sessions/` — saved session transcripts
- `~/.rs-agent/trust.json` — per-project tool-permission trust store
- `~/.rs-agent/AGENTS.md` — global instructions merged into every system prompt
- `~/.rs-agent/skills/` — user-global skills

Project-local equivalents (`.rs-agent/skills/`, `AGENTS.md`/`CLAUDE.md`, `.rs-agent/commands/`)
take precedence and are merged on top of the global ones.

## `reference/`

`reference/` contains local research material (notes on other agent/CLI projects) used while
building rs-agent. It's git-ignored and is not part of the crate — you don't need it to build,
test, or use rs-agent, and PRs shouldn't depend on anything in it.

## Reporting issues

Include: OS, `rustc --version`, provider/model used, the command you ran, and (if relevant) a
minimal repro. If the bug involves the RLM `repl` tool, include the Python snippet that triggered
it.
