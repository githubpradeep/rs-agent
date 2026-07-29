#!/usr/bin/env bash
set -euo pipefail
test -f hello.txt
# Exact single line
got="$(cat hello.txt | tr -d '\r')"
test "$got" = "hello-from-rs-agent"
echo "ok: hello.txt"
