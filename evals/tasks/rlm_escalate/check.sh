#!/usr/bin/env bash
# Pass if the agent recovered the buried token (and ideally used repl).
set -euo pipefail
TOKEN="$(cat .eval_token)"
LOG="${EVAL_AGENT_LOG:-}"

if [[ -z "$LOG" || ! -f "$LOG" ]]; then
  echo "FAIL: EVAL_AGENT_LOG missing"
  exit 1
fi

if ! grep -qF "$TOKEN" "$LOG"; then
  echo "FAIL: token $TOKEN not found in agent output"
  exit 1
fi

# Soft signal: prefer repl usage (warn but still pass if token found)
if ! grep -qE 'repl|load_file|\[rlm_escalate\]' "$LOG"; then
  echo "WARN: token found but no clear repl/escalate signal in log"
fi

echo "PASS: found $TOKEN"
exit 0
