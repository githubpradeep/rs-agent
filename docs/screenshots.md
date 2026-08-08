# TUI screenshot checklist

Reference steal UI surfaces to capture after Waves 1–5:

| Shot | How | What to show |
|------|-----|----------------|
| Idle chrome | fresh TUI | Header chip `○ idle`, brand-ish status, lean footer hints |
| Attention toast | trigger permission | Toast + `● blocked`; dismiss Esc/click |
| Help | `?` in normal mode | Filterable keymap overlay |
| Settings | `/settings` | Theme / mouse / toast / notify tabs |
| Palette | `Ctrl+K` | Fuzzy slash command list |
| Fleet panel | `/fleet` or `c` | Board: ACTIONS / WORKERS / WISHES / READY |
| Wish modal | City `w` | Compose wish overlay |
| Seat detail | Enter on worker | Status strip + dedicated log (FOLLOW chip) |
| Call tree | `/tree` mid-turn | TurnBar + Tree\|Timeline |
| Theme auto | `theme = "auto"` | Host light/dark via `COLORFGBG` |

Store PNGs under `docs/img/` when ready (not required for CI).
