#!/usr/bin/env bash
# RLM USP demo — grow a corpus, run a one-shot agent turn that must use `repl`.
#
# Usage:
#   ./scripts/demo-rlm.sh
#   ./scripts/demo-rlm.sh --prepare-only
#   ./scripts/demo-rlm.sh --provider anthropic --model claude-sonnet-4-20250514
#
# Requires: python3, a built rs-agent (or cargo), and a configured provider API key.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

PROVIDER="${PROVIDER:-}"
MODEL="${MODEL:-}"
PREPARE_ONLY=0
TARGET_BYTES="${TARGET_BYTES:-100000}"
CORPUS="$ROOT/example/rlm_demo_corpus.md"
SEED="$ROOT/example/rlm_long_doc.md"
BIN="${RS_AGENT_BIN:-$ROOT/target/release/rs-agent}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --prepare-only) PREPARE_ONLY=1; shift ;;
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

if [[ ! -f "$SEED" ]]; then
  echo "error: missing seed $SEED" >&2
  exit 1
fi

if ! command -v python3 >/dev/null 2>&1; then
  echo "error: python3 is required for the RLM repl" >&2
  exit 1
fi

echo "==> Preparing demo corpus (~${TARGET_BYTES} bytes) → $CORPUS"
python3 - "$SEED" "$CORPUS" "$TARGET_BYTES" <<'PY'
import sys
from pathlib import Path

seed_path, out_path, target = Path(sys.argv[1]), Path(sys.argv[2]), int(sys.argv[3])
seed = seed_path.read_text()
if "RLM-TREE-42" not in seed:
    raise SystemExit("seed must contain secret token RLM-TREE-42")

# Pad with numbered noise blocks; keep the seed (and token) exactly once at the end.
pad = []
n = 0
body = ""
while len((body := "\n".join(pad) + "\n\n" + seed).encode()) < target:
    n += 1
    pad.append(
        f"## Pad block {n}\n"
        "This is filler so the corpus is large enough that stuffing it into a "
        "single model context is wasteful. The agent should load it into the REPL "
        "`context` variable and slice/search in Python instead of pasting it all "
        "into the chat transcript.\n"
        + ("lorem " * 40)
    )

out_path.write_text(body)
print(f"wrote {out_path} ({out_path.stat().st_size} bytes, {n} pad blocks)")
PY

if [[ "$PREPARE_ONLY" -eq 1 ]]; then
  echo "prepare-only done."
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

PROMPT=$(cat <<EOF
You are running the rs-agent RLM demo. Follow these rules strictly:

1. Use the \`repl\` tool. Do NOT use \`read\` to dump the whole file into chat.
2. In Python: load_file('example/rlm_demo_corpus.md') into context (or set context).
3. Print len(context) and the first 200 characters only.
4. Find the secret token (it looks like RLM-…) by searching/slicing context in Python — do not guess.
5. Call llm_query once with a short prompt to summarize ONLY the Rivers and Mountains sections (use a slice of context, not the whole pad).
6. Call FINAL with a JSON-like result: token + one-paragraph summary.
7. Your final user-visible answer must include the exact secret token.
EOF
)

echo "==> Running: $BIN ${ARGS[*]} -p '…'"
echo "    (corpus: $(wc -c < "$CORPUS" | tr -d ' ') bytes)"
echo

set +e
OUT="$("$BIN" "${ARGS[@]}" -p "$PROMPT" 2>&1)"
STATUS=$?
set -e

echo "$OUT"
echo
echo "==== demo result ===="
if echo "$OUT" | grep -q 'RLM-TREE-42'; then
  echo "PASS: secret token RLM-TREE-42 appeared in the output."
else
  echo "FAIL: token RLM-TREE-42 not found — model may have skipped repl or hallucinated."
  echo "Retry with a stronger model, e.g.:"
  echo "  ./scripts/demo-rlm.sh --provider anthropic --model claude-sonnet-4-20250514"
fi
exit "$STATUS"
