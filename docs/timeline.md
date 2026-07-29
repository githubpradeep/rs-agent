# Session timeline

Fork a conversation from any API message index — useful when a turn went wrong and you want a
clean branch without replaying the whole session.

## Commands

| Command | Effect |
|---------|--------|
| `/timeline` | Toggle the timeline side panel (API message index + preview) |
| `/fork` | Fork the full transcript into a new session id |
| `/fork @N` | Truncate to the first N API messages, then fork |
| `/fork @N label` | Same, with a branch label in the session title |
| `/fork label` | Full-transcript fork with a branch label |

In the timeline panel (normal mode):

- `j` / `k` or ↑/↓ — move selection
- `Enter` — fork at the selected `@N`
- `Esc` — close the panel

Forked sessions keep `parent_id` and appear in `/sessions` with a title suffix like
`[experiment]` or `[@3]`.
