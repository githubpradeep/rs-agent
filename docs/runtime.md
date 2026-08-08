# Runtime control plane

rs-agent can publish a herdr-style lifecycle bus and (on Unix) serve a JSON-lines control socket.

## Lifecycle

Written to `~/.rs-agent/lifecycle.json` and available in-process:

| State | Meaning |
|-------|---------|
| `blocked` | Permission, question, or stuck |
| `working` | Thinking / tool running |
| `done` | Turn finished (unseen until focused) |
| `idle` | Ready |

```bash
rs-agent status show
rs-agent status wait --until blocked --timeout-secs 120
rs-agent status resume --seat Fleet-1 --answer "approved, continue"
```

## Daemon

```bash
rs-agent runtime serve              # listens on ~/.rs-agent/rs-agent.sock (0o600)
rs-agent api ping
rs-agent api agent.status
rs-agent api agent.wait --params '{"until":"blocked","timeout_ms":30000}'
rs-agent api agent.steer --params '{"seat":"Fleet-1","text":"summarize"}'
rs-agent runtime stop
```

`PROTOCOL_VERSION` is `1`. Set `RS_AGENT_SOCKET` / `RS_AGENT_RUNTIME=1` for the ops skill.

## Notifications

Config (`~/.rs-agent/config.toml`):

```toml
toast = true
toast_sound = false
notify = "off"   # off | terminal | system
```

Terminal uses OSC9 / Kitty OSC99; system uses `osascript` / `notify-send`. Notifications are suppressed while the TUI is focused.

## Orchestration extras

- `beads fail` classification: retriable vs terminal (`beads::fail` / `reopen_for_retry`)
- Gate waits: note `wait_until:<rfc3339>` → marshal `ungate_due`
- Routing handoff: `/route <seat> [reason]` (allow-list `allowed_transitions`)
- Priority queue: `.rs-agent/ready-queue.json` (`queue` module)
- Schedules: `rs-agent schedule add …` / `due`
- PLAN_EXECUTE MVP: `rs-agent plan-execute "1. …\n2. …"`
