# rs-agent TODO

Productionize **and** make it pleasant to use day-to-day — then share.
RLM stays the USP; UI / skills / ergonomics are what make strangers stick around.

## Done (foundation + Wave 1 + Wave 2 + Wave 3)

- [x] Core agent loop + streaming tools + RLM REPL / call tree
- [x] File/web tools + `repl`
- [x] Sessions, compaction, token tracking, context files
- [x] Abort + steer; API retries; thinking display
- [x] Config: `~/.rs-agent/config.toml` + project overrides (CLI wins)
- [x] Skills v1 + prompt templates + 5 starter skills in `skills/`
- [x] TUI slash: `/help` `/keys` `/clear` `/context` `/commands` `/skills` `/skill` `/prompt` `/reload` `/mode` `/sessions` `/export` `/trust` `/model` `/compact` `/new` `/tree` `/theme` `/rename` `/history`
- [x] Modes: `plan` / `ask` / `agent` (tool filtering)
- [x] Input history (Up/Down); `t` thinking toggle; `G` jump bottom
- [x] Status bar: model · mode · depth · YOLO · session · tree
- [x] Dangerous-bash heuristics + huge tool-result truncation
- [x] Session titles + `list_summaries` + markdown export helper
- [x] Trust store `list` / `clear`
- [x] LICENSE (MIT), Cargo metadata, CI, CONTRIBUTING, docs/skills + keymap
- [x] README rewrite (RLM + TUI + skills)
- [x] First-launch wizard (TTY; writes real `config.toml`)
- [x] Clear “no API key” / “python3 missing” errors with exact fix commands
- [x] Bracketed paste; `/history`; `@file` / `#dir` pickers; `/rename`
- [x] Richer permission cards (once / always / deny)
- [x] Collapsible tool blocks; tool spinner + elapsed
- [x] Live `/tree` side panel; REPL stdout panel; soft themes; configurable keybindings
- [x] Tab-complete `/skill` `/prompt`; `/reload` refreshes system prompt; `/context` toggle
- [x] `/model` interactive picker (aliases + live `fetch_models`)
- [x] Mid-session provider+model switch (pi parity: `/model`, `/provider`, Ctrl+P)
- [x] OpenAI / Bedrock thinking parity; default thinking budget when supported
- [x] Call tree persistence + RLM smoke tests + example gallery
- [x] Release workflow + install one-liner (`scripts/install.sh`)
- [x] `reference/` git-ignored / documented

---

## A. Day-1 UX (remaining)

### Sessions
- [ ] Optional session branch / fork — *after* flat sessions feel solid

---

## C. Skills & workflows (remaining)

- [x] Tab-complete template / skill names in insert mode
- [x] Hot-reload that also refreshes system prompt context (`/reload`)
- [x] `/context` toggle rules on/off for a session

---

## B. TUI product surface (remaining)

_(Wave 3 delivered named themes + single-char keybindings; richer variants deferred.)_

## D. RLM polish

- [ ] Integration tests: REPL → `llm_query` / `agent_query` → `FINAL` (needs a live provider)
- [x] RLM smoke test: `CallTree` register/snapshot/breadcrumb (no network) + REPL exec smoke test (skips if no `python3`)
- [x] Persist call tree in session JSON (`SessionData::call_tree`; `/tree` falls back to last saved snapshot summary when idle)
- [x] Stream REPL stdout into TUI
- [ ] Optional Docker/isolated REPL sandbox — deferred
- [x] Example gallery beyond `rlm_long_doc.md` (`example/README.md`, `example/rlm_mapreduce.md`)

---

## E. Providers

- [x] `/model` interactive picker (list from provider)
- [x] Model aliases from config used in `/model`
- [x] Mid-session **provider + model** switch (pi parity): registry, `/model provider/id`, `/provider`, `Ctrl-P` cycle
- [x] Pi-style **static model catalog** (~1000 models) — picker shows catalog for ready providers
- [x] `/provider` / `/login` interactive picker + paste API key + open console/signup URL (`~/.rs-agent/secrets.toml`)
- [x] Persist last `/model`/`/provider` selection to `~/.rs-agent/config.toml` (restored on restart)
- [x] OpenAI / Bedrock thinking parity
- [x] Mark `opencode-cli` experimental in TUI banner (and already noted in README provider table)

---

## F. Ship / share

- [x] Release binaries (macOS/Linux) via `.github/workflows/release.yml` on `v*` tags
- [x] Install one-liner (curl script pointing at release artifacts)
- [x] Keep `reference/` out of published artifact (git-ignored, untracked; documented in README/CONTRIBUTING)
- [ ] Tag `v0.1.0` + release notes — *manual; ask before tagging*
- [ ] Dogfood with 3 outsiders → `v0.2`

---

## H. Later (v0.2+)

- [ ] Windows support
- [ ] Session fork/share
- [ ] Docker/isolated REPL sandbox
- [ ] Thin Rust extension hooks (not full TS plugin VM)
- [ ] RPC/SDK embed
- [ ] OAuth flows
- [ ] Package/marketplace only if skills outgrow git folders

---

## Still out of scope (for now)

Swarm YAML DAGs, maximal oh-my-pi IDE/LSP/DAP surface, telemetry product, cloning pi’s entire extension marketplace before skills work.
