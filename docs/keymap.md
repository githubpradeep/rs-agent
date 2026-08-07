# TUI keymap reference

rs-agent's TUI uses three input modes, shown in the input box border/title:

- **NORMAL** — navigate/scroll; not editing the prompt
- **INSERT** — typing a message or a `/command`
- **WAITING** — the agent is running a turn; input is paused

All single-key bindings below are the defaults and can be remapped — see
[Configurable keybindings](#configurable-keybindings).

## Normal mode

| Key | Action |
|-----|--------|
| `i` | Switch to insert mode |
| `q` | Quit |
| `t` | Toggle the most recent assistant message's thinking trace open/closed |
| `e` | Toggle the most recent tool result block open/closed |
| `T` | Toggle the live call-tree side panel (see below) |
| `G` | Jump to the bottom of chat (resume auto-follow) |
| `↑` | Scroll up one line |
| `↓` | Scroll down one line |
| `Page Up` | Scroll up 10 lines |
| `Page Down` | Scroll down 10 lines |

## Insert mode

| Key | Action |
|-----|--------|
| `Enter` | Submit the current input. If it starts with `/`, it's run as a slash command instead of sent to the model. |
| `@` | Open the fuzzy file picker (see below) |
| `#` | Open the fuzzy directory picker (attaches a non-recursive listing) |
| `Tab` | Complete or open a picker for `/skill <name>` / `/prompt <name>` / `/p <name>` (see below) |
| `Esc` | Return to normal mode |
| `Backspace` | Delete the previous character |
| any character | Insert into the prompt |

### File picker (`@`)

Press `@` in **insert or normal** mode. A file list overlay appears above the input
(status bar shows how many files were scanned). Type to fuzzy-filter, `↑`/`↓` to move,
`Enter`/`Tab` to insert the path, `Esc` to cancel.

If you see “(no files found…)”, the walk found nothing under the current working directory
(check you launched rs-agent from the project root).

### Directory picker (`#`)

Same UX as `@`, but picks a directory and inserts `#path` (expanded to a short listing
when you submit the message).

### Skill / prompt tab-complete (`Tab`)

Typing `/skill <partial>`, `/prompt <partial>`, or `/p <partial>` and pressing `Tab`:

- If exactly one skill/template name matches the partial text, it's completed in place.
- If several match, a picker opens (same navigation keys as the file picker above) listing the
  candidates so you can pick one with `↑`/`↓` + `Enter`/`Tab`.

### Model picker (`/model`)

Running `/model` with no arguments opens an interactive **cross-provider** picker (pi-style):

- Seeded from the **built-in static catalog** (~1000 models) for every provider that has credentials
  configured — same rule as pi (“only models from configured providers”)
- Export keys like `OPENROUTER_API_KEY` / `ANTHROPIC_API_KEY` to unlock those catalog slices
- Background live `fetch_models` merges any extra IDs the API returns
- **Fuzzy filter** as you type (subsequence + multi-word tokens) — e.g. `sonnet`, `ocs4`, `deep flash`
- Selecting an entry switches **provider and model** mid-session when needed (no restart)

`/model <name-or-alias>` and `/model <provider>/<model>` work non-interactively.
Successful switches are saved to `~/.rs-agent/config.toml` and restored on the next start
(unless you pass `--provider` / `--model`).

### Provider picker (`/provider` / `/login`)

Running `/provider` (or `/login`) with no arguments opens an interactive provider menu:

- Each row shows `[ready]` or `[needs key]`, plus a console/signup URL when available
- Select a **ready** provider to switch immediately (default model for that provider)
- Select a **needs key** provider to open its signup/console URL in the browser, then paste
  the API key in the TUI (masked). Keys are saved to `~/.rs-agent/secrets.toml` (chmod 600)
  and exported into the matching env var for the session
- `/provider list` prints auth status without opening the picker
- `/provider <name>` switches or starts the connect flow for that name

Secrets do not overwrite env vars that are already set.

## Waiting mode (agent is running a turn)

| Key | Action |
|-----|--------|
| `Esc` | **Abort** the in-flight turn (and any running RLM subtree) |
| `Enter` | **Steer** — queue your typed message as a follow-up once the current turn finishes, without waiting for it to fully complete first |

## Fleet attach (TUI takeover)

While a headless fleet worker runs in the background:

| Slash command | Action |
|---------------|--------|
| `/city` | Seat board (all workers) |
| `/seat follow <seat>` | Live formatted log tail; worker keeps running |
| `/seat steer <text>` | Steer followed worker without attach |
| `/seat abort` | Abort followed/attached turn |
| `/seat open <seat>` | Inspect session read-only (no pause) |
| `/seat attach <seat>` | Pause worker, load its session, chat as that seat |
| `/seat detach` or `/detach` | Save session and resume the background worker |

`/fleet …` aliases work for follow/attach/detach/logs. Footer chip shows `FOLLOW` / `ATTACHING` / `ATTACHED` / `INSPECT`. Quitting the TUI while attached auto-resumes the worker. Esc / Enter steer work as in Waiting mode once you are attached and a turn is running.

## Global (any mode)

| Key | Action |
|-----|--------|
| `Ctrl-C` | Quit immediately |
| `Ctrl-P` | Cycle to the next ready `provider/model` (pi-style) |
| Mouse click on a `💭 ...` line | Toggle that message's thinking trace open/closed |
| Mouse click on a `⚙ ...` / `⚠ ...` line | Toggle that tool result block open/closed |
| Mouse scroll | Scroll chat history |

Mouse capture is on by default (so the app can handle those clicks/scrolls), which
means plain drag-to-select is taken by the TUI. **Shift+drag** still selects text in
most terminals (Terminal.app, iTerm2, Alacritty, Kitty, …). To turn mouse capture off
entirely and restore normal selection/scroll, set `disable_mouse = true` in
`~/.rs-agent/config.toml` (keyboard toggles `t` / `e` and PgUp/PgDn still work).

## Collapsible tool result blocks

Each tool call's result renders as its own collapsible block under the message that triggered
it, instead of being dumped inline as chat text:

- Collapsed: `⚙ tool_name — first 100 chars of output… (click/e to expand)` (⚠ instead of ⚙ for
  errors, in red)
- Expanded: the full result text, with a header you can click (or press `e`) to collapse again

New tool results always start collapsed; toggle with `e` (last block on the last assistant
message) or by clicking the block's header line.

## Tool-in-progress spinner

While a tool call is running, the status bar shows an animated spinner frame (`| / - \`) plus
the tool name and elapsed seconds, e.g. `⠋ bash (2.3s)`, so long-running tools (shell commands,
`repl`, web fetches) don't look stalled. It clears automatically when the tool result, an error,
`Done`, or `Aborted` arrives.

## Live `/tree` side panel

`/tree` (or the `T` key in normal mode) toggles a right-hand side panel showing the full Deep Context
call tree (root → agent/llm/repl sub-calls, with `…`/`✓`/`✗`/`⊘` status markers), not just the
one-line breadcrumb in the status bar. The panel auto-opens when `repl` starts; status bar shows
`[D]` while Deep Context is active. The panel updates live as sub-calls spawn and finish; when
idle with no active run it falls back to the last saved snapshot for the session, if any.

## REPL output panel

While the `repl` tool (RLM Python REPL) is executing, a short panel above the input box streams
its live stdout/stderr (stderr lines prefixed with `!`), capped to the most recent ~8KB, so you
can watch progress instead of waiting for the final tool result block.

## Soft themes

`/theme [dark|light|forest]` switches the TUI color palette at runtime (used consistently across
chat, panels, pickers, and the permission prompt); with no argument it prints the current theme.
Set a persistent default via `theme = "dark"` (or `"light"` / `"forest"`) in
`~/.rs-agent/config.toml`.

## Configurable keybindings

The single-key bindings shown throughout this doc (`insert`, `quit`, `toggle_thinking`,
`jump_bottom`, `expand_tool`, `toggle_tree`, `perm_once`, `perm_always`, `perm_path`, `perm_deny`) can be
remapped in `~/.rs-agent/config.toml`:

```toml
[keybindings]
insert = "i"
quit = "q"
toggle_thinking = "t"
jump_bottom = "G"
expand_tool = "e"
toggle_tree = "T"
perm_once = "a"
perm_always = "t"
perm_path = "p"
perm_deny = "d"
```

Only entries you set are overridden; anything omitted keeps its default. Run `/keys` in the TUI
to see the bindings actually in effect for the current session.

## Permission prompts

When a tool needs approval (anything not auto-allowed under `-a`/`--approve` or `--auto-mode`, and
not already covered by an already-trusted project **or a path-scoped allow**), a prompt overlay appears showing the tool
name, a pretty-printed (or raw, if not JSON) preview of its input truncated to a sensible size,
and — for commands flagged as risky (e.g. destructive shell commands) — a red `DANGEROUS` warning
with the reason:

| Key | Action |
|-----|--------|
| `a` / `Enter` | Allow this one call only |
| `p` | Path allow — remember this tool under the file's parent directory (`~/.rs-agent/permissions.json`) |
| `t` | Trust this project — allow this and future calls here without prompting again |
| `d` / `Esc` | Deny |

(These letters follow the `perm_once` / `perm_path` / `perm_always` / `perm_deny` bindings above, so they move
if you remap those actions.)

Project trust lives in `~/.rs-agent/trust.json`; path-scoped allows in `~/.rs-agent/permissions.json`.
`/trust list` shows both. See the README's CLI options table for `-a`/`--approve` (skip all prompts)
and `--auto-mode` (AUTO: file tools + reads; still prompts for bash/repl) — status bar shows
`YOLO` vs `AUTO` respectively.

## Slash commands

Typed in insert mode, run on `Enter`.

| Command | Description |
|---------|-------------|
| `/help` | List commands and key hints |
| `/keys` | Show the active keymap (reflects any remapping) |
| `/clear` | Clear the visible chat transcript (session data is kept) |
| `/context [on\|off]` | Show loaded `AGENTS.md`/`CLAUDE.md`/commands, or toggle whether they're included in the system prompt (rebuilds it live) |
| `/commands` | List project commands under `.rs-agent/commands/` |
| `/tree` | Toggle the live call-tree side panel and show a breadcrumb/summary |
| `/skills` | List discovered skills (see `docs/skills.md`) |
| `/skill <name>` | Inject a skill's instructions into the conversation (tab-completes) |
| `/prompt <name> [args]` / `/p <name> [args]` | Render a prompt template into the input (tab-completes) |
| `/reload` | Re-discover skills/templates and rebuild the system prompt from scratch |
| `/mode plan\|ask\|agent` | Switch agent interaction mode (tool filtering) |
| `/model [name]` | With no args, opens an interactive model picker (aliases + live provider fetch); with a name/alias, switches immediately |
| `/provider [name]` / `/login` | Interactive provider menu: switch ready providers, or open signup URL + paste API key into `~/.rs-agent/secrets.toml` |
| `/theme [dark\|light\|forest]` | Switch (or show) the TUI color theme |
| `/compact` | Compact/summarize the conversation to reclaim context |
| `/new` | Start a fresh session (new session id) |
| `/fork [label]` | Fork current session (same messages; new id + parent) |
| `/sessions` | List saved sessions (shows fork parent when present) |
| `/export [md\|json\|html]` | Export the current session transcript (markdown default; json/html for sharing) |
| `/trust list\|reset` | Manage the per-project trust store |
| `/rename <title>` | Rename the current session |
| `/history [query\|n]` | Browse/search/recall input history |

If a command isn't recognized, the TUI responds with `Unknown command: ... (try /help)`.
