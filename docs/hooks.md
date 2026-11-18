# Hooks (plugin seed)

rs-agent can run small scripts around tool calls, user messages, and goals.
Drop executables (or `.sh` / `.py` scripts) into:

1. `~/.rs-agent/hooks/` — user-global
2. `.rs-agent/hooks/` — project-local (overrides home on name clash)

| Script | When | Args / stdin | Behavior |
|--------|------|--------------|----------|
| `before_tool` / `before_tool.sh` | Before a tool runs | argv: `tool_name`, `tool_input_json` | Non-zero exit **blocks** the tool; stderr is shown as the error |
| `after_tool` / `after_tool.sh` | After a tool finishes | argv: `tool_name`, `is_error` (`0`\|`1`); stdin: result text | Best-effort; failures are ignored |
| `on_message` / `on_message.sh` | When a user turn starts | stdin: user message | Best-effort; failures are ignored |
| `before_goal_continue` | Before `/goal` auto-continue | stdin: goal condition | Non-zero exit **pauses** the goal |
| `on_goal_achieved` | When a goal is marked achieved | stdin: condition + reason | Advisory |

Timeout: 5 seconds per hook.

## Example — block `bash` outside trusted commands

`.rs-agent/hooks/before_tool.sh`:

```bash
#!/usr/bin/env bash
tool="$1"
input="$2"
if [[ "$tool" == "bash" ]]; then
  echo "bash blocked by project before_tool hook" >&2
  exit 1
fi
exit 0
```

```bash
chmod +x .rs-agent/hooks/before_tool.sh
```

## Example — gate long-running goals on CI

`.rs-agent/hooks/before_goal_continue.sh`:

```bash
#!/usr/bin/env bash
# Exit 0 to allow continue; non-zero pauses /goal (e.g. wait for green CI).
exit 0
```

Hooks are optional — if the directory is missing, nothing loads.
