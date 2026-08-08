#!/usr/bin/env bash
# Public Deep Context demo — realistic outage log, natural prompt (not a CTF).
#
# Usage:
#   ./scripts/demo-deep-context.sh
#   ./scripts/demo-deep-context.sh --prepare-only
#   ./scripts/demo-deep-context.sh --tui          # print TUI instructions + prepare corpus
#   ./scripts/demo-deep-context.sh --provider anthropic --model claude-sonnet-4-20250514
#
# Requires: python3, release binary (or cargo), provider API key.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

PROVIDER="${PROVIDER:-}"
MODEL="${MODEL:-}"
PREPARE_ONLY=0
TUI_ONLY=0
TARGET_BYTES="${TARGET_BYTES:-180000}"
LOG="$ROOT/example/demo/outage.log"
GEN="$ROOT/scripts/gen_outage_log.py"
BIN="${RS_AGENT_BIN:-$ROOT/target/release/rs-agent}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --prepare-only) PREPARE_ONLY=1; shift ;;
    --tui) TUI_ONLY=1; shift ;;
    --provider) PROVIDER="$2"; shift 2 ;;
    --model) MODEL="$2"; shift 2 ;;
    --bytes) TARGET_BYTES="$2"; shift 2 ;;
    --bin) BIN="$2"; shift 2 ;;
    -h|--help)
      sed -n '2,12p' "$0"
      exit 0
      ;;
    *)
      echo "unknown arg: $1" >&2
      exit 1
      ;;
  esac
done

if ! command -v python3 >/dev/null 2>&1; then
  echo "error: python3 is required" >&2
  exit 1
fi

echo "==> Generating realistic outage log (~${TARGET_BYTES} bytes) → $LOG"
python3 "$GEN" -o "$LOG" --bytes "$TARGET_BYTES"

if [[ "$PREPARE_ONLY" -eq 1 || "$TUI_ONLY" -eq 1 ]]; then
  echo
  if [[ "$TUI_ONLY" -eq 1 ]]; then
    cat <<EOF
==> TUI recording steps
1. cargo run --release -- -a --provider ${PROVIDER:-anthropic}
2. Press i (insert), paste this prompt:

We had elevated checkout errors on 2026-03-15. Look at example/demo/outage.log
and tell me: (1) when the incident started, (2) the root cause, (3) what was a
red herring. Be specific — cite timestamps and config values from the log.

3. Watch for [rlm_escalate] / repl / [D] — then open /tree
4. Good answer mentions DB_POOL_MAX=2 (or pool max 2) after the 14:31 deploy,
   503s from ~14:33, and that Stripe was a red herring.

Talk track: docs/demo.md
EOF
  else
    echo "prepare-only done."
  fi
  exit 0
fi

if [[ ! -x "$BIN" ]]; then
  echo "==> Building release binary…"
  cargo build --release
  BIN="$ROOT/target/release/rs-agent"
fi

ARGS=(-a --timeout 600)
if [[ -n "$PROVIDER" ]]; then
  ARGS+=(--provider "$PROVIDER")
fi
if [[ -n "$MODEL" ]]; then
  ARGS+=(--model "$MODEL")
fi

# Natural user prompt — do NOT prescribe repl / FINAL / tool order.
PROMPT=$(cat <<'EOF'
We had elevated checkout errors on 2026-03-15. Look at example/demo/outage.log and tell me: (1) when the incident started, (2) the root cause, (3) what was a red herring. Be specific — cite timestamps and config values from the log.
EOF
)

echo "==> Running: $BIN ${ARGS[*]} -p '…'"
echo "    log: $(wc -c < "$LOG" | tr -d ' ') bytes, $(wc -l < "$LOG" | tr -d ' ') lines"
echo

set +e
OUT="$("$BIN" "${ARGS[@]}" -p "$PROMPT" 2>&1)"
STATUS=$?
set -e

echo "$OUT"
echo
echo "==== demo checks (heuristic) ===="
PASS=1
check() {
  local label="$1"
  local pattern="$2"
  if echo "$OUT" | grep -Eiq "$pattern"; then
    echo "OK   $label"
  else
    echo "MISS $label  (pattern: $pattern)"
    PASS=0
  fi
}

check "pool / DB_POOL mentioned" 'DB_POOL_MAX|pool_max|max_size[= ]?2|pool (size|max).*2'
check "afternoon deploy window" '14:3[1-4]|2\.14\.0|deploy'
check "503 / timeout cascade" '503|pool_timeout|starvation|exhaust'
check "stripe called out as red herring OR discounted" 'stripe|red.?herring|not (a |the )?payment|payment.?provider'

if echo "$OUT" | grep -Eiq 'repl|rlm_escalate|load_file|\[D\]'; then
  echo "OK   deep-context path visible in output (repl/escalate)"
else
  echo "NOTE deep-context markers not obvious in -p text (normal); use --tui to show /tree"
fi

echo
if [[ "$PASS" -eq 1 ]]; then
  echo "PASS: answer looks like it found the real outage story."
else
  echo "WEAK: answer may have missed the root cause — retry with a stronger model:"
  echo "  ./scripts/demo-deep-context.sh --provider anthropic --model claude-sonnet-4-20250514"
  echo "Or record interactively: ./scripts/demo-deep-context.sh --tui"
fi
exit "$STATUS"
