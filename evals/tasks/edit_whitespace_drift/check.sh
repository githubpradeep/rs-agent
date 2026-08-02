#!/usr/bin/env bash
set -euo pipefail
out=$(python3 greet.py)
test "$out" = "Hello, world"
grep -q 'f"Hello' greet.py || grep -q "f'Hello" greet.py
