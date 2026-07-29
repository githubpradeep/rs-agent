#!/usr/bin/env bash
set -euo pipefail
# Agent output is not always saved; check the captured agent log for the token.
# The harness exports EVAL_AGENT_LOG pointing at the full -p transcript.
test -n "${EVAL_AGENT_LOG:-}"
grep -q 'RLM-EVAL-91C2' "$EVAL_AGENT_LOG"
echo "ok: rlm needle token in agent output"
