#!/usr/bin/env bash
set -euo pipefail
test -f found.txt
grep -q 'NEEDLE_TOKEN_9f3a' found.txt
