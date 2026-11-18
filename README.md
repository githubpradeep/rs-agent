# rs-agent

**A lean terminal coding agent with Deep Context.**  
Large logs and repos stay in a persistent Python REPL — the agent peeks,
slices, and runs `llm_query` / `agent_query` as a call **tree**, so huge
context never blows the model window.

**One line:** one binary, your shell, your provider — and big context that
fits.

## Why this, not another chat wrapper

- **Deep Context when the file is too big.** Large docs load into a persistent
  `python3` REPL once. The agent reads slices, then `llm_query(prompt)` /
  `agent_query(task)` run as a **tree** — parents see summaries, not subtree
  dumps. Huge `read`s auto-escalate (`[rlm_escalate]` → `repl`). Status shows
  `[D]`; `/tree` visualizes the live call tree.
  Inspired by [Recursive Language Models](https://arxiv.org/abs/2512.24601).
- **The daily loop you already know.** read / edit / write / bash / grep / find /
  web, permission prompts, sessions, skills. Drop markdown in `skills/` or
  `~/.rs-agent/skills/` ([`docs/skills.md`](docs/skills.md)).
- **Lean by design.** Two provider clients (Anthropic + OpenAI-compatible),
  a curated catalog, no daemons, no control planes, no C-library highlighting
  deps. Harness exit gates and crash recovery keep long turns honest.

**Trust:** `repl` is unsandboxed `python3`. Read [`docs/trust.md`](docs/trust.md)
before running with `-a`.

## Quickstart (Anthropic)

Requirements: a stable Rust toolchain and `python3` on `PATH` (Deep Context `repl`).

```sh
git clone https://github.com/githubpradeep/rs-agent.git
cd rs-agent
export ANTHROPIC_API_KEY=sk-ant-...
cargo run --release -- --provider anthropic
```

Prebuilt install (macOS Apple Silicon / Linux x86_64) **after a `v*` GitHub
release exists**:

```bash
curl -fsSL https://raw.githubusercontent.com/githubpradeep/rs-agent/main/scripts/install.sh | bash
```

Until [releases](https://github.com/githubpradeep/rs-agent/releases) list a tag,
that curl line will 404 — use source.

One-shot / scripting:

```sh
cargo run --release -- -p "summarize example/rlm_long_doc.md via repl"
cargo run --release -- --mode json -p "list files with ls"
```

Deep Context demo (realistic ~180KB outage log):

```sh
./scripts/demo-deep-context.sh --provider anthropic
# Interactive (best for recording): ./scripts/demo-deep-context.sh --tui
```

## Configuration

Layered config: `~/.rs-agent/config.toml` (user) plus `.rs-agent/settings.toml` /
`.rs-agent.toml` (project). CLI flags always win.

```toml
# ~/.rs-agent/config.toml
provider = "anthropic"
model = "claude-sonnet-4-20250514"
approve = true          # YOLO — see docs/trust.md
auto_mode = false
rlm_depth = 2
rlm_escalate_chars = 10000   # auto Deep Context escalate on huge reads
thinking_budget = 10000
max_iterations = 99999
timeout = 300
auto_goal = true             # task-like requests keep working until verified done (/goal clear to disable)
verify_cmd = "cargo test"    # harness runs this before letting the agent stop; failure re-injects
disable_mouse = false   # true = native text selection; Shift+drag also works with mouse on
theme = "dark"          # dark | light | forest

[model_aliases]
fast = "claude-haiku-4-20250514"
smart = "claude-opus-4-20250514"

[keybindings]
# Remap single-key actions (defaults shown):
# insert = "i"
# quit = "q"
# toggle_thinking = "t"
# jump_bottom = "G"
# expand_tool = "e"
# toggle_tree = "T"
# perm_once = "a"
# perm_always = "t"
# perm_deny = "d"
```

State under `~/.rs-agent/`:

| Path | Purpose |
|------|---------|
| `~/.rs-agent/sessions/` | Saved session transcripts (`--resume <id>` / `--list-sessions`) |
| `~/.rs-agent/trust.json` | Per-project "always allow" tool-permission store |
| `~/.rs-agent/AGENTS.md` | Global instructions merged into every system prompt |
| `~/.rs-agent/skills/` | User-global skills |
| `~/.rs-agent/secrets.toml` | Pasted API keys from `/login` |

Project-local `AGENTS.md` / `CLAUDE.md` (walked up from cwd) and
`.rs-agent/commands/*.md` are merged into the system prompt unless
`--no-context-files`.

## Skills

A skill is a markdown file (optionally with YAML frontmatter: `name`,
`description`, `triggers`) that teaches a workflow. Drop one in:

- `~/.rs-agent/skills/*.md` — every project
- `.rs-agent/skills/*.md` — project-local
- `skills/*.md` at a repo root — this repo ships a few starters

```
/skills            # list
/skill pr-review   # inject into the conversation
```

Authoring: [`docs/skills.md`](docs/skills.md). Hooks: [`docs/hooks.md`](docs/hooks.md).

## TUI

```
cargo run --release
```

| Key | Mode | Action |
|-----|------|--------|
| `i` | Normal | Enter insert mode |
| `Esc` | Insert (idle) | Back to normal mode |
| `Esc` | Waiting (agent running) | **Abort** the current turn |
| `Enter` | Waiting (agent running) | **Steer** — queue a follow-up for the next turn |
| `Enter` | Insert | Submit message / run a `/command` |
| `@` | Insert | Fuzzy file picker |
| `↑` `↓` | Insert | Cycle input history |
| `↑` `↓` `PgUp` `PgDn` | Normal | Scroll chat history |
| `t` | Normal | Toggle thinking trace |
| `e` | Normal | Toggle last tool result |
| `T` | Normal | Toggle Deep Context call-tree side panel |
| `s` | Normal | Toggle sessions panel |
| `G` | Normal | Jump to bottom |
| `a` / `Enter` | Permission prompt | Allow once |
| `t` | Permission prompt | Trust this project |
| `d` / `Esc` | Permission prompt | Deny |
| `^P` | Any | Cycle provider/model |
| `^C` | Any | Quit |

Core slash commands:

| Command | Description |
|---------|--------------|
| `/help` | Commands and key hints |
| `/tree` | Deep Context call tree |
| `/model` `/provider` `/login` | Switch model or paste a key |
| `/goal [condition]` | Auto-continue until a condition is met |
| `/recall <id>` | Read an archived (compacted) step |
| `/compact` `/new` `/fork` | Session hygiene |
| `/sessions` | Parallel project chats |
| `/mode plan\|ask\|agent` | Tool permissions |

Full keymap: [`docs/keymap.md`](docs/keymap.md).

### CLI options

| Flag | Description |
|------|-------------|
| `--provider` | `anthropic` (recommended), `openai`, `opencode` |
| `--model` | Model override |
| `--rlm-depth` | Max Deep Context recursion depth (default 2) |
| `--rlm-escalate-chars` | Auto Deep Context escalate threshold (default 10000) |
| `--thinking-budget` | Extended-thinking token budget (Anthropic); `0` disables |
| `--mode` | `-p` output: `text` (default) or `json` |
| `-p, --prompt` | One-shot prompt (non-interactive) |
| `-a, --approve` | **YOLO.** Skip permission prompts. See [`docs/trust.md`](docs/trust.md). |
| `--auto-mode` | Auto-approve read-only tools only |
| `-r, --resume <id>` | Resume a saved session |
| `--list-sessions` | List saved sessions and exit |
| `--list-models` | List models for the provider and exit |
| `--no-context-files` | Skip `AGENTS.md`/`CLAUDE.md`/project-command discovery |
| `--system-prompt` | Override the default system prompt |
| `--append-system-prompt` | Append text (or `@path/to/file`); repeatable |
| `--max-iterations` | Cap on agent loop iterations per turn (default 99999) |
| `--api-key` / `--api-key-env` | Supply or redirect the API key |
| `--base-url` | Override the provider API base URL (any OpenAI-compatible endpoint) |
| `--timeout` | Request timeout in seconds (default 300) |

`cargo run --release -- --help` for the current flag set.

## Providers

| Provider | Flag / id | Auth env | Notes |
|----------|-----------|----------|-------|
| Anthropic | `anthropic` | `ANTHROPIC_API_KEY` | Recommended. |
| OpenAI | `openai` | `OPENAI_API_KEY` | Any OpenAI-compatible endpoint via `--base-url`. |
| OpenRouter | `openrouter` | `OPENROUTER_API_KEY` | Aggregator — a curated slice of frontier models (`openrouter/<id>`, incl. `openrouter/auto`). |
| OpenCode Zen | `opencode` | `OPENCODE_API_KEY` | Reuses `~/.local/share/opencode/auth.json` when present. |

Curated catalog in [`data/models.catalog.json`](data/models.catalog.json).
`/model` shows models for providers that have credentials. Mid-session:
`/model`, `Ctrl-P`, `/provider` / `/login`. Last selection is saved to
`~/.rs-agent/config.toml`.

## Deep Context workflow

1. Put a large payload into the REPL's `context` (or `load_file` / `load_dir`).
2. Peek/chunk in Python; `llm_query(prompt)` (leaf) or `agent_query(task)` (nested agent).
3. Finish with `FINAL(value)`.
4. Inspect `/tree` (auto-opens while `repl` runs; `[D]` in the status bar).

Worked example: [`example/rlm_long_doc.md`](example/rlm_long_doc.md).

```
 Call tree
 root [running] summarize corpus
 ├─ repl [running] load + peek
 │  ├─ llm [done] extract section A
 │  ├─ llm [done] extract section B
 │  └─ agent [running] reconcile
 │     └─ llm [done] FINAL draft
 └─ …
```

```
User → Root AgentLoop → tools (incl. repl)
                           ↓
                     Python REPL (persistent)
                           ↓
          llm_query (leaf) / agent_query (nested AgentLoop)
                           ↓
                     CallTree (abort cancels subtree)
```

## Evals

`evals/` holds end-to-end agent tasks (edit drift, long output, RLM needle,
escalate, webfetch). Run one:

```sh
./scripts/eval-agent.sh --provider anthropic --only rlm_escalate
```

## `reference/`

Local research notes (other agent/CLI projects). Git-ignored — not part of the crate.

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md).

## License

[MIT](LICENSE)
