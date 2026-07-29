#!/usr/bin/env bash
set -euo pipefail
test -f PAGE.txt
grep -q 'Example' PAGE.txt
echo "ok: PAGE.txt"
