# rs-agent

**Your everyday coding agent — with Deep Context.**  
A Rust TUI for the work you already do (edit, bash, search, sessions, skills), plus a
persistent-REPL core so big context doesn’t blow the window when the task gets hard.

## Why rs-agent

- **Built for daily coding.** Same loop you expect from a modern terminal agent: read / edit /
  write / bash / grep / find / web tools, vim-flavored TUI, streaming, permissions, sessions,
  `/model` + `/provider`, skills and prompt templates.
- **Better when context gets big (the USP).** Deep Context keeps large docs, repos, and diffs in a
  persistent Python REPL *outside* the model window. The agent peeks and slices in code, then
  calls `llm_query` / `agent_query` as a **tree** — parents only see summaries, not full
  subtree dumps. Inspired by [Recursive Language Models](https://arxiv.org/abs/2512.24601).
  Everyday tasks stay flat and fast; hard context tasks escalate into the tree without you
  changing tools. Huge `read`s auto-escalate (`[rlm_escalate]` → `repl`). Status bar shows `[D]`
  and `/tree` opens automatically while Deep Context runs.
- **Skills.** Drop a markdown file with frontmatter into `skills/` or `~/.rs-agent/skills/` and
  the agent picks up a new reusable workflow — optional `tools:` allow-list (Skills 2.0). See
  [`docs/skills.md`](docs/skills.md). Hooks: [`docs/hooks.md`](docs/hooks.md).

**One line:** Everyday coding agent with Deep Context — load 100KB of source once, query it 100
times, never hit a limit.

## Quickstart (Anthropic)

Install a prebuilt binary:

```bash
curl -fsSL https://raw.githubusercontent.com/githubpradeep/rs-agent/main/scripts/install.sh | bash
```

Or build from source:

```sh
git clone https://github.com/githubpradeep/rs-agent.git
cd rs-agent
export ANTHROPIC_API_KEY=sk-ant-...
cargo run --release -- --provider anthropic
```

One-shot / scripting:

```sh
# One-shot prompt, plain text output
cargo run --release -- --provider anthropic -p "summarize example/rlm_long_doc.md via repl"

# One-shot prompt, JSON event stream (for scripting/tooling)
cargo run --release -- --provider anthropic --mode json -p "list files with ls"

# USP demo (~100KB corpus → repl → llm_query → FINAL). See docs/demo.md
./scripts/demo-rlm.sh --provider anthropic
```

Requirements: a stable Rust toolchain and `python3` on `PATH` (used by the Deep Context `repl` tool).

## Configuration

Today, rs-agent reads layered config from `~/.rs-agent/config.toml` (user) plus
`.rs-agent/settings.toml` / `.rs-agent.toml` (project overrides). CLI flags always win.

```toml
# ~/.rs-agent/config.toml
provider = "anthropic"
model = "claude-sonnet-4-20250514"
approve = true
auto_mode = false
rlm_depth = 2
rlm_escalate_chars = 10000   # auto Deep Context escalate on huge reads
thinking_budget = 10000
max_iterations = 100
timeout = 300
base_url = "https://api.anthropic.com/v1"
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

State that's already live under `~/.rs-agent/`:

| Path | Purpose |
|------|---------|
| `~/.rs-agent/sessions/` | Saved session transcripts (`--resume <id>` / `--list-sessions`) |
| `~/.rs-agent/trust.json` | Per-project "always allow" tool-permission store |
| `~/.rs-agent/AGENTS.md` | Global instructions merged into every system prompt |
| `~/.rs-agent/skills/` | User-global skills (see below) |

Project-local `AGENTS.md` / `CLAUDE.md` (walked up from cwd) and `.rs-agent/commands/*.md` are
also discovered automatically and merged into the system prompt unless `--no-context-files` is
passed.

## Skills

A skill is a markdown file (optionally with YAML frontmatter: `name`, `description`, `triggers`)
that teaches the agent a specific workflow — debugging, PR review, writing a commit message, the
Deep Context long-doc pattern, etc. Drop one in:

- `~/.rs-agent/skills/*.md` — available in every project
- `.rs-agent/skills/*.md` — project-local, shared via your repo
- `skills/*.md` at a repo root — shipped alongside a project (this repo ships a handful of
  starter skills this way)

Then, in the TUI:

```
/skills            # list discovered skills
/skill pr-review   # inject a skill's instructions into the conversation
```

**Status:** discovery, frontmatter parsing, and TUI `/skills` / `/skill <name>` are live. See
[`docs/skills.md`](docs/skills.md) for the full authoring guide.

## TUI

```
cargo run --release -- --provider anthropic
```

| Key | Mode | Action |
|-----|------|--------|
| `i` | Normal | Enter insert mode |
| `Esc` | Insert (idle) | Back to normal mode |
| `Esc` | Waiting (agent running) | **Abort** the current turn |
| `Enter` | Waiting (agent running) | **Steer** — queue a follow-up message for the next turn |
| `Enter` | Insert | Submit message / run a `/command` |
| `@` | Insert | Open fuzzy file picker; `↑`/`↓` navigate, `Enter`/`Tab` select, `Esc` cancel |
| `↑` `↓` | Insert | Cycle input history (older/newer submitted messages) |
| `↑` `↓` `PgUp` `PgDn` | Normal | Scroll chat history |
| `t` | Normal | Toggle the most recent assistant message's thinking trace |
| `e` | Normal | Toggle the most recent tool result block open/closed |
| `G` | Normal | Jump to the bottom of chat (resume auto-follow) |
| click 💭 | Any | Toggle a message's thinking trace open/closed |
| click ⚙/⚠ | Any | Toggle a tool result block open/closed |
| `a` / `Enter` | Permission prompt | Allow this tool call once |
| `t` | Permission prompt | Trust this project (auto-allow here going forward) |
| `d` / `Esc` | Permission prompt | Deny |
| `^P` | Any | Cycle provider/model (ready providers with credentials) |
| `^C` | Any | Quit |

Slash commands (type in insert mode, `Enter` to run):

| Command | Description |
|---------|--------------|
| `/help` | List available commands and key hints |
| `/compact` | Summarize/compact the conversation to free context |
| `/new` | Start a new session |
| `/model [provider/model\|alias]` | Interactive cross-provider picker, or switch mid-session (pi-style) |
| `/provider [name]` / `/login` | Provider menu: switch ready providers, or open signup URL + paste API key (`~/.rs-agent/secrets.toml`) |
| `/tree` | Toggle the Deep Context call-tree side panel |
| `/timeline` | Toggle API-message timeline; Enter forks at `@N` ([`docs/timeline.md`](docs/timeline.md)) |
| `/fork [@N] [label]` | Fork session (optionally truncated at API index N) |
| `/image <path>` | Queue Kitty-protocol image display |
| `/lsp start\|stop\|status` | Minimal diagnostics via rust-analyzer ([`docs/lsp.md`](docs/lsp.md)) |
| `/skill-pack export\|import` | Zip share/import for skills |
| `/skills`, `/skill <name>` | List / inject a skill |
| `/prompt` `/p <name> [args]` | Fill a prompt template into the input |
| `/mode plan\|ask\|agent` | Switch tool permissions (read-only / no tools / full) |
| `/keys` `/clear` `/context` `/sessions` `/export [md\|json\|html]` `/trust` | UX helpers (see `/help`) |

Full reference: [`docs/keymap.md`](docs/keymap.md).

### CLI options

| Flag | Description |
|------|-------------|
| `--provider` | `anthropic` (recommended), `openai`, `opencode`, `opencode-cli` (experimental), `bedrock` |
| `--model` | Model override |
| `--rlm-depth` | Max Deep Context recursion depth, root → child → leaf (default 2) |
| `--rlm-escalate-chars` | Char threshold for auto Deep Context escalate on huge reads (default 10000) |
| `--thinking-budget` | Extended-thinking token budget (Anthropic); `0` disables |
| `--mode` | Output mode for `-p`: `text` (default) or `json` |
| `-p, --prompt` | One-shot prompt (non-interactive) |
| `-a, --approve` | **YOLO mode.** Skip permission prompts entirely — every tool call auto-executes. Use with care. |
| `--auto-mode` | Lighter YOLO: auto-approve only read-only tools (`read`/`grep`/`ls`/`find`/`webfetch`/`websearch`); everything else still prompts |
| `-r, --resume <id>` | Resume a saved session |
| `--list-sessions` | List saved sessions and exit |
| `--list-models` | List models available for the chosen provider and exit |
| `--no-context-files` | Skip `AGENTS.md`/`CLAUDE.md`/project-command discovery |
| `--system-prompt` | Override the default system prompt entirely |
| `--append-system-prompt` | Append text (or `@path/to/file`) to the system prompt; repeatable |
| `--max-iterations` | Cap on agent loop iterations per turn (default 100) |
| `--api-key` / `--api-key-env` | Supply or redirect the provider API key |
| `--base-url` | Override the provider's API base URL |
| `--timeout` | Request timeout in seconds (default 300) |

Run `cargo run --release -- --help` for the exact, current flag set.

## Providers

| Provider | Flag / id | Auth env | Notes |
|----------|-----------|----------|-------|
| Anthropic | `anthropic` | `ANTHROPIC_API_KEY` | Recommended. |
| OpenAI | `openai` | `OPENAI_API_KEY` | |
| OpenRouter | `openrouter` | `OPENROUTER_API_KEY` | Aggregator — hundreds of catalog models once keyed. |
| Groq / DeepSeek / Together / Fireworks / xAI / … | same id | see `/provider` | OpenAI-compatible; listed from the built-in catalog. |
| AWS Bedrock | `bedrock` / `amazon-bedrock` | `~/.aws/credentials` or env | Newer models need inference-profile IDs (`us.anthropic…`); bare IDs are auto-prefixed from your AWS region. |
| OpenCode (REST) | `opencode` | `OPENCODE_API_KEY` | |
| OpenCode CLI | `opencode-cli` | (local CLI) | Experimental. `/model` lists whatever `opencode models` returns (full OpenCode catalog). |

rs-agent ships a **pi-style static model catalog** (~1000 models / ~35 providers in
[`data/models.catalog.json`](data/models.catalog.json), synced from `reference/pi`).
`/model` only shows models for providers that have credentials configured (same rule as pi).
Export e.g. `OPENROUTER_API_KEY` to unlock the large OpenRouter slice. Refresh the catalog with
`python3 scripts/sync-model-catalog.py` when `reference/pi` is present.

Mid-session: `/model`, `Ctrl-P`, `/provider` (or `/login`) switch across providers — no restart.
The last provider/model is written to `~/.rs-agent/config.toml` and restored next launch
(override with `--provider` / `--model`).
For providers without a key yet, `/provider` opens the console/signup URL and lets you paste
an API key into `~/.rs-agent/secrets.toml` (also exported to the matching env var for the process).

## Deep Context workflow

1. Put a large payload into the REPL's `context` (or `load_file` / `load_dir`).
2. Run Python that peeks/chunks the context and calls `llm_query(prompt)` (leaf LM call) or
   `agent_query(task)` (recursive sub-agent with its own tools).
3. Finish with `FINAL(value)`.
4. Inspect the call tree with `/tree` in the TUI (auto-opens while `repl` runs; status bar shows
   `[D]`), or `tree`/`tree_final` events in `--mode json`.

See [`example/rlm_long_doc.md`](example/rlm_long_doc.md) for a worked example.

Example `/tree` panel while Deep Context is running (status bar shows `[D]`):

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

## `reference/`

`reference/` holds local research notes (other agent/CLI projects) used while building rs-agent.
It's git-ignored and isn't part of the crate — you don't need it to build, run, or contribute.

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for build/test/lint commands and PR expectations.

## License

[MIT](LICENSE)
