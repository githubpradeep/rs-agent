# LSP diagnostics (minimal)

rs-agent can talk to a language server over stdio and show a compact error/warn count in the
status bar. This is diagnostics-only — no code actions or jump-to-definition yet.

## Setup

1. Install a server on `PATH`. Default: `rust-analyzer`.
2. Override with `RS_AGENT_LSP=/path/to/server` if needed.
3. In the TUI: `/lsp start`

| Command | Effect |
|---------|--------|
| `/lsp start` | Spawn the language server for the current working directory |
| `/lsp stop` | Tear it down |
| `/lsp status` | Print the current footer summary |

After `write` / `edit` / `apply_patch` on a known language file (`.rs`, `.ts`, `.py`, …), rs-agent
sends `textDocument/didOpen` + `didSave`. Publish-diagnostics notifications update the footer as
`LSP E:n W:m` (or `LSP✓` when clean).
