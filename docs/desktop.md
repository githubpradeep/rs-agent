# Desktop app (deferred client)

**Canonical interactive surface:** the terminal TUI ([ui-ia.md](ui-ia.md)).
Desktop is a **thin viewer/client** of the same durable runtime — not a second product.

## Why deferred

- Adoption for coding agents is terminal-native (Claude Code / Herdr pattern).
- The TUI covers chat, Deep Context, sessions, permissions, and City supervision.
- A desktop wrapper adds packaging/updater without unlocking new agent capabilities
  until the control socket covers City jobs.

## Likely shape later

1. **Thin host** — Tauri window that attaches to `~/.rs-agent/rs-agent.sock`
   (see [runtime.md](runtime.md)), not a rewrite of the agent loop.
2. **Reuse** — session store, fleet status, beads/wishes, tools stay in Rust;
   desktop is chrome + notifications + optional file dialogs.
3. **No dual-stack tools** — avoid a separate JS tool runtime.
4. **No browser ops dashboard as primary** — web City cockpit is explicitly out
   of v1 (splits the product and weakens SSH/overnight).

Until then, run `rs-agent` in a modern terminal (Kitty, WezTerm, Ghostty, or iTerm2).
