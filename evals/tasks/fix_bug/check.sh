#!/usr/bin/env bash
set -euo pipefail
test -f math_util.py
out="$(python3 math_util.py)"
test "$out" = "OK"
# Ensure the +1 bug is gone
python3 -c 'from math_util import add; assert add(2,3)==5'
echo "ok: math_util.py"
