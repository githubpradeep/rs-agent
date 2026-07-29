#!/usr/bin/env bash
# ~90KB corpus with a buried token — tempting to read() into chat.
set -euo pipefail
TOKEN="RLM-ESC-$(printf '%04d' $((RANDOM % 10000)))"
{
  echo "# Large corpus (eval)"
  echo
  echo "This file is intentionally large. Prefer REPL over dumping into chat."
  echo
  # ~90KB of padding
  python3 - <<'PY'
pad = ("lorem ipsum dolor sit amet consectetur adipiscing elit. " * 40 + "\n") * 80
print(pad, end="")
PY
  echo
  echo "## Buried section"
  echo "The secret evaluation token is: ${TOKEN}"
  echo
  python3 - <<'PY'
print(("more padding xyz " * 50 + "\n") * 20, end="")
PY
} > corpus.md
echo "$TOKEN" > .eval_token
# sanity: size
wc -c corpus.md | awk '{print "corpus_bytes="$1}'
