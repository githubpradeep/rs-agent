#!/usr/bin/env bash
set -euo pipefail
out=$(python3 calc.py)
test "$out" = "OK"
grep -q 'return a + b' calc.py
