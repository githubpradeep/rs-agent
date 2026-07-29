# Desktop app (deferred)

PLAN.md lists a desktop shell (Electron or Tauri) as a P4 stretch goal. **Not implemented in
v0.1** — the product surface is the terminal UI.

## Why deferred

- The TUI already covers the core loop (tools, Deep Context, sessions, permissions).
- A desktop wrapper adds packaging, updater, and OS permission surface area without unlocking new
  agent capabilities.
- Prefer polishing Kitty images, LSP, and session timeline in-terminal first.

## Likely shape later

If/when it ships:

1. **Thin host** — Tauri (or Electron) window embedding the same agent core over a local protocol,
   not a rewrite of the agent loop.
2. **Reuse** — session store, tools, and Deep Context stay in the Rust library; the desktop layer
   is chrome + optional file dialogs / notifications.
3. **No dual-stack tools** — avoid maintaining a separate JS tool runtime.

Until then, run `rs-agent` in a modern terminal (Kitty, WezTerm, Ghostty, or iTerm2).
