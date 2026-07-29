#!/usr/bin/env bash
set -euo pipefail
test -f FOUND.txt
got="$(tr -d '[:space:]' < FOUND.txt)"
test "$got" = "RS_AGENT_EVAL_7F3A"
echo "ok: FOUND.txt"
