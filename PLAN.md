# rs-agent: Gap Analysis & Roadmap

**What we execute now:** [`docs/productize.md`](docs/productize.md) (overnight
factory + Deep Context, freeze City offices, ship v0.1). This file is the older
pi / OpenCode feature-gap list — mostly landed; do not use it as the backlog.

Based on thorough comparison against **pi** (badlogic/pi-mono) and **opencode**
(anomalyco/opencode). See `reference/` for the full source dumps.

---

## 1. The USP — Deep Context (née RLM)

**What it is:** A persistent Python REPL where the agent loads large context
*once*, then makes `llm_query()` leaf calls and `agent_query()` sub-agent calls
against it. 100KB files never blow the context window because the parent only
sees summaries, not subtree dumps.

**Why it's unique:**
| Competitor | Equivalent? |
|------------|-------------|
| pi | No. Closest is `orchestrator` (experimental, spawns child pi processes via RPC) |
| opencode | No. `task` tool spawns child sessions but no persistent REPL, no call tree, no incremental context slicing |

**Why nobody sees it:**
1. **Name problem** — "RLM" sounds like academic jargon. Users hear "I have to
   learn something" instead of "context handled automatically".
2. **Not default** — only triggers when you know to use `repl` + `llm_query`.
3. **No screenshot** — the README has text but no `/tree` panel screenshot.
4. **Hidden panel** — `/tree` exists but doesn't auto-open when RLM is active.
5. **No "wow" demo** — the one-shot works but it's not obvious that *without*
   RLM the agent would have cratered.

---

## 2. Gap Summary — Where rs-agent Lags

### 🔴 Agent Loop & Tools

| Gap | pi | opencode | Fix |
|-----|----|----------|-----|
| No `edit` diff output | Unified diff output | Unified diff output | Add `diff` crate → annotate edit tool result with unified diff |
| No `apply_patch` | No (edit only) | Yes | Add `apply_patch` tool (search/replace from unified diff) |
| No `question` tool | Extension | Yes | Add `QuestionTool` (reads from stdin, returns user answer) |
| No `todowrite` tool | Extension | Yes | Add `TodoWriteTool` (in-memory list, persisted to session) |
| No subagents (beyond agent_query) | Extension API | build/plan/general/explore | Expose `agent_query` as a built-in tool (not just from REPL) |
| No doom-loop guard | Loop guard in code | Last-3 identical detection | Bump from 2→3 identical calls before blocking |
| No streaming tool execution | Per-tool override | FiberSet parallel | Keep sequential for weak models; parallel when strong |
| No tool output bounding | - | File-backed overflow | Add overflow-to-file for >100KB tool results |

### 🔴 TUI — Biggest Gap

| Feature | pi | opencode | rs-agent | Effort |
|---------|----|----------|----------|--------|
| **Diff rendering** | Word-level intra-line | Word-level via `diff` lib | None | Med (~150 lines + `diff` crate) |
| **Autocomplete** | Multi-provider + fuzzy | File frecency | Tab-complete only | High (new component) |
| **Theme system** | JSON light/dark, hot-reload | Light/dark/system | 3 hardcoded themes | Low (it exists, add hot-reload) |
| **Command palette** | No | Yes (Ctrl+K, fuzzy) | No | High |
| **Permission diff preview** | No | Full overlay | None | Med |
| **Slash commands** | 21 (/export, /share, /login, /logout...) | 15+ | 10+ | Low (add more) |
| **Code block folding** | No | Yes (tool output collapse) | Tool blocks only | Med |
| **Cost display** | Per-model pricing table | Per-message + session total | Token count only | Med |
| **Session timeline** | Tree navigation | Fork from any message | Fork only | High |
| **Session sharing** | HTML export + HuggingFace | opencode.ai links | None | Med |
| **LSP status** | No | Yes (diagnostics + code actions) | No | Very high |
| **Image/PDF rendering** | Kitty protocol | Kitty + iTerm2 | None | Med |
| **Help dialog** | No | Yes (overlay) | Text /help | Low |
| **Model picker** | Yes (selector) | Yes (dialog) | Yes (slash command) | Low |

### 🔴 Extensibility

| Feature | pi | opencode | rs-agent |
|---------|----|----------|----------|
| Plugin/Extension API | 65+ examples, hooks everywhere | Hook-based, NPM packages, TUI slots | None |
| Custom agents | Via extension | opencode.json + AGENTS.md | Skills (weaker) |
| SDK | RPC mode + TypeScript API | sdk/ + sdk-next/ | None |
| MCP OAuth | No | Yes (3 transports) | Stdio only |

---

## 3. Action Plan — Prioritized

### P0 — Ship the USP (make it visible)

- [x] **Rename "RLM" → "Deep Context"** in README, status bar, `/help`,
  system prompt, CLI flags. Every user-facing string. Keep "RLM" in code.
- [x] **Auto-show `/tree` panel** when REPL is active (TUI change, ~50 lines).
- [x] **Add status indicator `[D]`** to status bar when Deep Context is
  running (breadcrumb line shows "repl>llm").
- [x] **Lower auto-escalation threshold** from ~32K chars to ~10K chars
  (`src/agent/rlm_escalate.rs`).
- [ ] **Add README screenshot** of the `/tree` panel showing a 15-node call
  hierarchy. Capture and commit.
- [x] **Add "why" one-liner** to the opener message in TUI: *"Deep Context:
  load big files once, query them 100 times, never hit a limit."*

### P1 — TUI Parity (high visibility)

- [x] **Add diff rendering** to edit tool results. Use `similar` crate
  (already in Cargo.lock?) or `diff` — compute unified diff between old/new
  file, render inline in the tool result block.
- [x] **Display diff in permission prompts** when the tool is `edit` — show
  the user what will change before they allow.
- [x] **Add `todowrite` tool** (~100 lines). In-memory todo list displayed
  in a status summary. Persisted in session JSON.
- [x] **Add `question` tool** (~80 lines). Prompts user via TUI overlay,
  returns answer. Use existing `PendingPermission` channel pattern.
- [x] **Add more slash commands**: `/export` (markdown transcript), `/fork`
  (exists but show visual feedback), `/sessions` (list + resume from TUI).
- [x] **Add cost tracking**: track input/output tokens per-turn, show in
  status bar / session metadata.

### P2 — Agent Loop Quality

- [x] **Bump doom-loop detection** from 2→3 identical calls
  (`src/agent/loop.rs`).
- [x] **Add streaming tool result display** — progressively render tool
  output as it streams (currently only `repl` has live output via
  `ReplOutput` event).
- [x] **Make `agent_query` a top-level tool** (not just available from
  the REPL). Expose it as `task` tool that spawns a sub-agent loop.
- [x] **Add input autocomplete** for `/skill`, `/prompt`, `/model` names
  (currently tab-complete only in specific contexts — generalize).

### P3 — Extensibility & Ecosystem

- [x] **Skills 2.0**: Allow skills to declare custom tools (not just
  system prompt text). Use YAML frontmatter `tools:` field.
- [x] **Session import/export**: JSON export (`/export`), HTML export for
  sharing. Store structured format with all tool results.
- [x] **Plugin seed**: Minimal hook system — `before_tool`, `after_tool`,
  `on_message` callbacks loaded from `.rs-agent/hooks/`.
- [x] **Add `apply_patch` tool** — accepts unified diff, applies to file.
  Reuses edit tool's internal logic. Useful for model-generated diffs.

### P4 — Stretch (v0.2+)

- [x] Session timeline visualization (fork from any message, navigate tree)
- [x] Image rendering in terminal (Kitty protocol)
- [x] LSP integration (diagnostics; code actions still future)
- [x] Desktop app (Electron wrapper, or Tauri) — **deferred**; see [`docs/desktop.md`](docs/desktop.md)
- [x] Plugin marketplace / skill sharing — skill-pack zip export/import (marketplace later)

---

## 4. Quick Wins (this week)

| What | Where | Lines |
|------|-------|-------|
| Rename "RLM" → "Deep Context" in UI | `prompts/system.md`, `src/tui/app.rs`, `README.md`, `docs/` | ~20 | ✅ |
| Auto-show `/tree` when REPL active | `src/tui/app.rs` — set `show_tree_panel = true` on `ToolUseStart` if `name == "repl"` | ~5 | ✅ |
| Add `[D]` status indicator | `src/tui/app.rs` — add `" [D]"` to breadcrumb when `tree_breadcrumb` has non-root active | ~5 | ✅ |
| Lower escalation threshold | `src/agent/rlm_escalate.rs` — change `escalate_chars` from 32000 to 10000 | ~5 | ✅ |
| Add `todowrite` tool | `src/tools/todowrite.rs` + register in `mod.rs` | ~100 | ✅ |
| Add `question` tool | `src/tools/question.rs` + wire through permission channel | ~80 | ✅ |
| Add diff rendering to edit | `Cargo.toml` + `similar` crate + render in `src/tui/app.rs` | ~150 | ✅ |
| `/export` markdown | `src/tui/app.rs` — serialise messages to markdown, write to file | ~60 | ✅ (pre-existing) |
| Cost tracking | `src/ai/token_count.rs` track per-model pricing, display in footer | ~100 | ✅ |

---

## 5. The One-Sentence Pitch

> **You close the laptop. Work continues. Big context never blows the window.**
> Local overnight factory (wish → workers → review) plus Deep Context for files
> that don't fit. Daily TUI is the on-ramp. See [`docs/productize.md`](docs/productize.md).
