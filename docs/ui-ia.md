# UI information architecture

Product decision: **terminal-native cockpit** (ratatui). HTML-style interaction
(overview + inspector + always-on composers) without a browser dashboard.
See the adoption research in the UI rethink plan: Claude Code / Herdr pattern.

## Surfaces

| Surface | Role |
|---------|------|
| **Chat** | Conversation with the focused session (or attached seat). Not an ops log. |
| **City** | Supervise the herd: wish intake, workers, flow, seat inspector. |
| **Sessions** | Parallel project chats. |
| **Tree** | Deep Context call tree (can coexist with City as a tab/strip). |
| **CLI + socket** | Canonical control plane; TUI is a client. |

Desktop/web ops dashboards are **not** v1. A future thin Tauri host may attach to
the same Unix socket — see [desktop.md](desktop.md).

## Focus zones

Keys are **zone-local**. Global bindings (`toggle_city`, `quit`, `?`) still apply.

| Zone | When active | Primary keys |
|------|-------------|--------------|
| `Chat` | Default / insert / waiting | typing, steer while waiting |
| `CityBoard` | City open, focus on workers/flow | ↑↓ select, Enter select, `u` spawn, `d` stop, `X` delete |
| `CityWish` | Wish composer focused | type, Enter submit |
| `CityInspector` | Seat selected | `f` follow, `a` attach, `o` open, Enter steers composer |
| `CitySteer` | Steer composer focused | type, Enter send steer |
| `Sessions` | Sessions panel focused | ↑↓, Enter switch, `n` new |
| `Tree` | Tree strip focused | ↑↓ expand |

Tab / `` ` `` cycles focus among visible zones. Esc backs out one level
(composer → inspector → board → close City).

## City layout (split — overview never replaced)

```text
wish> …                         [↵]
WORKERS  [u spawn]
● Crew-1 …
◐ Fleet-2 …
FLOW  wish → ready → doing → done
◆ Softmax   ▸ auth
── Fleet-2 ──
log…
steer> …                        [↵]
[f]ollow [a]ttach [o]pen [b]abort
```

- Top: persistent wish composer.
- Middle: workers + pipeline/flow list.
- Bottom: always-mounted inspector (empty until selection).

## Chat is not ops log

Do **not** `push_system` fleet boards, follow tails, or marshal dumps into the
transcript. Prefer:

- City inspector status line / log viewport
- Toasts for discrete events (spawn done, blocked seat)
- Slash `/city board` when the user explicitly asks for a text dump

## Success path (zero slash)

`rs-agent` → `c` → type wish → spawn → select worker → follow in inspector → steer.
