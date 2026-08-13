# rs-agent

**A local overnight coding factory.**  
Leave a backlog; cheaper workers implement; you review in the morning. Deep
Context keeps huge logs and repos *out* of the model window.

**One line:** You close the laptop. Work continues. Big context never blows the window.

Daily edit/bash/search is the on-ramp (vim-flavored TUI, sessions, skills,
`/model`). The product is unattended implement-and-review on your machine — not
another chat sidebar.

## Why this, not another coding agent

- **Overnight, not just a session.** `wish` → ready queue (beads) → `fleet up`
  workers with leases → review in the morning. Crash recovery so a dropped
  provider stream does not wedge the graph. See [`docs/overnight.md`](docs/overnight.md).
- **Deep Context when the file is too big.** Large docs stay in a persistent
  Python REPL. The agent peeks and slices, then `llm_query` / `agent_query` as a
  **tree** — parents see summaries, not subtree dumps. Huge `read`s auto-escalate
  (`[rlm_escalate]` → `repl`). Status `[D]`; `/tree` opens while it runs.
  Inspired by [Recursive Language Models](https://arxiv.org/abs/2512.24601).
- **Same daily loop you already know.** read / edit / write / bash / grep / find /
  web, permissions, skills. Drop markdown in `skills/` or `~/.rs-agent/skills/`
  ([`docs/skills.md`](docs/skills.md)).

**Trust:** workers run YOLO; `repl` is unsandboxed `python3`. Read
[`docs/trust.md`](docs/trust.md) before `-a` or `fleet up`.

Operator cockpit (wish → spawn → follow): [`docs/overnight.md`](docs/overnight.md).
Standing roles and the full office map: [`docs/city-ops.md`](docs/city-ops.md).

## Quickstart (Anthropic)

Requirements: a stable Rust toolchain and `python3` on `PATH` (Deep Context `repl`).

**Build from source** (works today):

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

### Day-1 path

```text
rs-agent          # TUI
c                 # cockpit: wish composer + workers
type a wish ↵     # becomes a design/task bead
u                 # spawn workers (or: rs-agent -a fleet up --seats Fleet-1,Fleet-2)
select a worker   # follow logs; steer if needed
```

Morning: `/beads ready` and review closed notes. Soft goals like “keep
implementing…” will **not** finish while ready/open beads remain. Prefer
`/goal no open beads`.

### Demos

```sh
# Deep Context — realistic outage log (~180KB). See docs/demo.md
./scripts/demo-deep-context.sh --provider anthropic
# Interactive (best for recording): ./scripts/demo-deep-context.sh --tui

# Overnight (same project cwd; YOLO)
cargo run --release -- -a --provider anthropic -- wish "summarize the outage demo" --auto
cargo run --release -- -a fleet up --seats Fleet-1 --budget-minutes 30
```

One-shot / scripting:

```sh
cargo run --release -- --provider anthropic -p "summarize example/rlm_long_doc.md via repl"
cargo run --release -- --provider anthropic --mode json -p "list files with ls"
```

Talk track and recording checklist: [`docs/demo.md`](docs/demo.md).

## Overnight (short)

```bash
# Intake
rs-agent wish "add tests for the parser" --auto

# Night — YOLO, named seats, wall-clock budget
rs-agent -a fleet up --seats Fleet-1,Fleet-2 --budget-minutes 480

# Morning
rs-agent          # TUI: c  then inspect workers / /beads ready
rs-agent fleet down
```

Pipeline: `design` → close spawns `implement` → close spawns `review`
(`bead fail` reopens implement; `bead land` after a passed review).

Two fleet seats no longer share one dirty checkout: `fleet up` uses a git
worktree per seat (`.rs-agent/worktrees/`). Pass `--shared-worktree` to opt
out. Details: [`docs/overnight.md`](docs/overnight.md),
[`docs/trust.md`](docs/trust.md).

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

State under `~/.rs-agent/`:

| Path | Purpose |
|------|---------|
| `~/.rs-agent/sessions/` | Saved session transcripts (`--resume <id>` / `--list-sessions`) |
| `~/.rs-agent/trust.json` | Per-project "always allow" tool-permission store |
| `~/.rs-agent/AGENTS.md` | Global instructions merged into every system prompt |
| `~/.rs-agent/skills/` | User-global skills |
| `~/.rs-agent/seats/` | Named seats (persistent agent identity + diary) |
| `~/.rs-agent/secrets.toml` | Pasted API keys from `/login` |

Project-local `AGENTS.md` / `CLAUDE.md` (walked up from cwd) and
`.rs-agent/commands/*.md` are merged into the system prompt unless
`--no-context-files`. Work graph: `.rs-agent/beads.json`. Fleet:
`.rs-agent/fleet/`. More paths: [`docs/overnight.md`](docs/overnight.md).

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
cargo run --release -- --provider anthropic
```

| Key | Mode | Action |
|-----|------|--------|
| `i` | Normal | Enter insert mode |
| `c` | Normal | Cockpit (wish / workers / follow) |
| `Esc` | Insert (idle) | Back to normal mode |
| `Esc` | Waiting (agent running) | **Abort** the current turn |
| `Enter` | Waiting (agent running) | **Steer** — queue a follow-up for the next turn |
| `Enter` | Insert | Submit message / run a `/command` |
| `@` | Insert | Fuzzy file picker |
| `↑` `↓` | Insert | Cycle input history |
| `↑` `↓` `PgUp` `PgDn` | Normal | Scroll chat history |
| `t` | Normal | Toggle thinking trace |
| `e` | Normal | Toggle last tool result |
| `G` | Normal | Jump to bottom |
| `a` / `Enter` | Permission prompt | Allow once |
| `t` | Permission prompt | Trust this project |
| `d` / `Esc` | Permission prompt | Deny |
| `^P` | Any | Cycle provider/model |
| `^C` | Any | Quit |

Day-1 slash commands:

| Command | Description |
|---------|--------------|
| `/help` | Commands and key hints |
| `/wish <text>` | Intake a wish as a bead |
| `/city` / `/fleet` | Cockpit (workers, follow, spawn) |
| `/beads` / `/beads ready` | Work graph |
| `/tree` | Deep Context call tree |
| `/model` `/provider` `/login` | Switch model or paste a key |
| `/compact` `/new` `/fork` | Session hygiene |
| `/mode plan\|ask\|agent` | Tool permissions |

Full keymap: [`docs/keymap.md`](docs/keymap.md). Screenshot checklist:
[`docs/screenshots.md`](docs/screenshots.md) (`docs/img/` after you capture).

### CLI options

| Flag | Description |
|------|-------------|
| `--provider` | `anthropic` (recommended), `openai`, `opencode`, `opencode-cli` (experimental), `bedrock` |
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
| `--base-url` | Override the provider API base URL |
| `--timeout` | Request timeout in seconds (default 300) |

`cargo run --release -- --help` for the current flag set. Subcommands:
`worker`, `fleet`, `wish`, `marshal`, `role`, `status`, `api`, `runtime`,
`schedule` — overnight operators use [`docs/overnight.md`](docs/overnight.md).

## Providers

| Provider | Flag / id | Auth env | Notes |
|----------|-----------|----------|-------|
| Anthropic | `anthropic` | `ANTHROPIC_API_KEY` | Recommended. |
| OpenAI | `openai` | `OPENAI_API_KEY` | |
| OpenRouter | `openrouter` | `OPENROUTER_API_KEY` | Aggregator — catalog models once keyed. |
| Groq / DeepSeek / Together / Fireworks / xAI / … | same id | see `/provider` | OpenAI-compatible catalog. |
| AWS Bedrock | `bedrock` / `amazon-bedrock` | `~/.aws/credentials` or env | Newer models need inference-profile IDs (`us.anthropic…`). |
| OpenCode (REST) | `opencode` | `OPENCODE_API_KEY` | |
| OpenCode CLI | `opencode-cli` | (local CLI) | Experimental. |

Static catalog (~1000 models) in [`data/models.catalog.json`](data/models.catalog.json).
`/model` shows models for providers that have credentials. Mid-session:
`/model`, `Ctrl-P`, `/provider` / `/login`. Last selection is saved to
`~/.rs-agent/config.toml`.

## Deep Context workflow

1. Put a large payload into the REPL's `context` (or `load_file` / `load_dir`).
2. Peek/chunk in Python; `llm_query(prompt)` (leaf) or `agent_query(task)` (nested agent).
3. Finish with `FINAL(value)`.
4. Inspect `/tree` (auto-opens while `repl` runs; `[D]` in the status bar).

Worked example: [`example/rlm_long_doc.md`](example/rlm_long_doc.md). Demo log:
[`docs/demo.md`](docs/demo.md).

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

Local research notes (other agent/CLI projects). Git-ignored — not part of the crate.

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md). Product sequence:
[`docs/productize.md`](docs/productize.md).

## License

[MIT](LICENSE)
