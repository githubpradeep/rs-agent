# TUI screenshot checklist

Product shots for README (`docs/img/` — capture locally; not required for CI):

| File | How | Must show |
|------|-----|-----------|
| `docs/img/tree.png` | Deep Context mid-turn, `/tree` | `[D]` + call tree, not the whole log in chat |
| `docs/img/idle.png` | fresh TUI | Opener: overnight factory + Deep Context |
| `docs/img/cockpit.png` | `c` then follow a worker | wish composer + worker heartbeat (optional) |

Record the 90s talk track in [`demo.md`](demo.md). Until PNGs exist, README uses the ASCII `/tree` sample.

Other surfaces (after Waves 1–5):

| Shot | How | What to show |
|------|-----|----------------|
| Idle chrome | fresh TUI | Header chip `○ idle`, brand-ish status, lean footer hints |
| Attention toast | trigger permission | Toast + `● blocked`; dismiss Esc/click |
| Help | `?` in normal mode | Filterable keymap overlay |
| Settings | `/settings` | Theme / mouse / toast / notify tabs |
| Palette | `Ctrl+K` | Fuzzy slash command list |
| Workers cockpit | `c` | wish composer + workers/flow + inspector |
| Follow | Enter worker · `f` | Log in inspector; chat stays clean |
| Cockpit + tree | Deep Context mid-turn | Tree drawer under cockpit |
| Call tree | `/tree` mid-turn | TurnBar + Tree\|Timeline |
| Theme auto | `theme = "auto"` | Host light/dark via `COLORFGBG` |
