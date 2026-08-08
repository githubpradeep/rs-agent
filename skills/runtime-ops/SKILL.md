---
name: runtime-ops
description: Operate the rs-agent control plane (lifecycle wait, fleet steer, notifications) when RS_AGENT_RUNTIME=1 or RS_AGENT_SOCKET is set.
---

# rs-agent runtime ops

Only use these commands when `RS_AGENT_RUNTIME=1` or `RS_AGENT_SOCKET` is set in the environment.
Do not invent socket paths; prefer `$RS_AGENT_SOCKET`.

## Status

```bash
rs-agent status show
rs-agent status wait --until blocked --timeout-secs 60
```

## Socket API

```bash
rs-agent api ping
rs-agent api agent.status
rs-agent api agent.wait --params '{"until":"blocked","timeout_ms":30000}'
rs-agent api agent.steer --params '{"seat":"Fleet-1","text":"pause and summarize"}'
rs-agent api notification.show --params '{"title":"rs-agent","body":"needs you","mode":"terminal"}'
```

## Daemon

```bash
rs-agent runtime serve
rs-agent runtime stop
```

## Safety

- Prefer `--no-focus` style ops: never steal the user's active TUI input.
- Wait for `blocked` before assuming a human must answer.
- After steer/abort, re-check `agent.status`.
