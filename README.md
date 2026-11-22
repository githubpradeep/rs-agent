# rs-agent

**A nimble, minimal terminal coding agent.**  
One Rust binary. Your shell. Your models. No daemons, no control planes, no
bloat — just a focused agent loop with Deep Context when files get huge.

**One line:** OpenRouter (or Anthropic / OpenAI) + a lean TUI that stays out of
the way.

## Why this

- **Nimble by design.** Curated tool catalog (read / edit / write / bash / grep /
  find / web / repl / task). Permission prompts, sessions, skills. No city
  fleets, no factory layers — a single agent that does the job.
- **OpenRouter-first.** One key unlocks Claude, GPT, Gemini, DeepSeek, Qwen,
  Grok, and more. Prompt-caching markers + sticky `session_id` keep cache warm;
  hits show in the header chip. `openrouter/auto` routes for you.
- **Deep Context when the file is too big.** Large docs load into a persistent
  `python3` REPL once. The agent peeks and slices; `llm_query` / `agent_query`
  run as a **tree** so parents see summaries, not dumps. Huge `read`s
  auto-escalate (`[rlm_escalate]` → `repl`). Status shows `[D]`; `/tree`
  visualizes the live call tree.
  Inspired by [Recursive Language Models](https://arxiv.org/abs/2512.24601).
- **Daily loop you already know.** Drop markdown in `skills/` or
  `~/.rs-agent/skills/` ([`docs/skills.md`](docs/skills.md)). Optional MCP
  servers and LSP diagnostics when you want them.

**Trust:** `repl` is unsandboxed `python3`. Read [`docs/trust.md`](docs/trust.md)
before running with `-a`.

## Quickstart (OpenRouter)

Requirements: a stable Rust toolchain and `python3` on `PATH` (Deep Context
`repl`).

```sh
git clone https://github.com/githubpradeep/rs-agent.git
cd rs-agent
export OPENROUTER_API_KEY=sk-or-...
cargo run --release -- --provider openrouter
```

Pick a model mid-session with `/model`, or pin one up front:

```sh
cargo run --release -- --provider openrouter --model anthropic/claude-sonnet-4.5
cargo run --release -- --provider openrouter --model openrouter/auto
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
cargo run --release -- --provider openrouter -p "summarize example/rlm_long_doc.md via repl"
cargo run --release -- --mode json -p "list files with ls"
```

Deep Context demo (realistic ~180KB outage log):

```sh
./scripts/demo-deep-context.sh --provider openrouter
# Interactive (best for recording): ./scripts/demo-deep-context.sh --tui
```

### Other providers

```sh
export ANTHROPIC_API_KEY=sk-ant-...
cargo run --release -- --provider anthropic

export OPENAI_API_KEY=sk-...
cargo run --release -- --provider openai
# Any OpenAI-compatible endpoint:
cargo run --release -- --provider openai --base-url https://your-endpoint/v1
```

## Providers

| Provider | Flag / id | Auth env | Notes |
|----------|-----------|----------|-------|
| **OpenRouter** | `openrouter` | `OPENROUTER_API_KEY` | **Recommended.** One key → many models (`anthropic/…`, `openai/…`, `google/…`, `deepseek/…`, `openrouter/auto`, free tiers). Prompt-caching breakpoints + sticky `session_id`; cache hits in the header chip. |
| Anthropic | `anthropic` | `ANTHROPIC_API_KEY` | Native Messages API + extended thinking. |
| OpenAI | `openai` | `OPENAI_API_KEY` | Any OpenAI-compatible endpoint via `--base-url`. |

Curated catalog in [`data/models.catalog.json`](data/models.catalog.json).
`/model` shows models for providers that have credentials. Mid-session:
`/model`, `Ctrl-P`, `/provider` / `/login`. Last selection is saved to
`~/.rs-agent/config.toml`.

Paste a key without leaving the TUI: `/login` → writes
`~/.rs-agent/secrets.toml`.

## Configuration

Layered config: `~/.rs-agent/config.toml` (user) plus `.rs-agent/settings.toml` /
`.rs-agent.toml` (project). CLI flags always win.

```toml
# ~/.rs-agent/config.toml
provider = "openrouter"
model = "anthropic/claude-sonnet-4.5"
approve = true          # YOLO — see docs/trust.md
auto_mode = false
rlm_depth = 2
rlm_escalate_chars = 10000   # auto Deep Context escalate on huge reads
thinking_budget = 10000      # Anthropic extended thinking; ignored elsewhere
max_iterations = 99999
timeout = 300
auto_goal = true             # task-like requests keep working until verified (/goal clear to disable)
verify_cmd = "cargo test"    # harness runs this before letting the agent stop
disable_mouse = false        # true = native text selection; Shift+drag also works with mouse on
theme = "dark"               # dark | light | forest | auto

[model_aliases]
fast = "google/gemini-3.5-flash"
smart = "anthropic/claude-opus-4.6"
auto = "openrouter/auto"

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

# Optional MCP stdio servers — see docs/mcp.md
# [[mcp.servers]]
# name = "filesystem"
# command = "npx"
# args = ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
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
MCP: [`docs/mcp.md`](docs/mcp.md). LSP: [`docs/lsp.md`](docs/lsp.md).

## TUI

```
cargo run --release -- --provider openrouter
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
| `--provider` | `openrouter` (recommended), `anthropic`, `openai` |
| `--model` | Model override (e.g. `anthropic/claude-sonnet-4.5`, `openrouter/auto`) |
| `--rlm-depth` | Max Deep Context recursion depth (default 2) |
| `--rlm-escalate-chars` | Auto Deep Context escalate threshold (default 10000) |
| `--thinking-budget` | Extended-thinking token budget (Anthropic); `0` disables |
| `--mode` | `-p` output: `text` (default) or `json` |
| `-p, --prompt` | One-shot prompt (non-interactive) |
| `-a, --approve` | **YOLO.** Skip permission prompts. See [`docs/trust.md`](docs/trust.md). |
| `--auto-mode` | Auto-approve reads + file edits; still prompt for bash/repl |
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
./scripts/eval-agent.sh --provider openrouter --only rlm_escalate
```

## `reference/`

Local research notes (other agent/CLI projects). Git-ignored — not part of the crate.

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md).

## License

[MIT](LICENSE)
