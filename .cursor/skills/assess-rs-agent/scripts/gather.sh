#!/usr/bin/env bash
# Snapshot metrics for assess-rs-agent. Run from repo root.
set -euo pipefail

root="$(cd "$(dirname "$0")/../../../.." && pwd)"
cd "$root"

echo "=== rs-agent assess snapshot ==="
echo "date:    $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo "root:    $root"
echo "branch:  $(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo n/a)"
echo "head:    $(git log -1 --format='%h %ad %s' --date=short 2>/dev/null || echo n/a)"
echo "commits: $(git rev-list --count HEAD 2>/dev/null || echo n/a)"
echo "first:   $(git log --reverse --format='%h %ad %s' --date=short 2>/dev/null | head -1)"
echo "status:  $(git status -sb 2>/dev/null | head -1)"
echo "tags:    $(git tag -l | tr '\n' ' ' | sed 's/[[:space:]]*$//')"
echo "tags_n:  $(git tag -l | wc -l | tr -d ' ')"
echo "crate:   $(grep -E '^version' Cargo.toml | head -1)"
echo "rust_n:  $(git ls-files '*.rs' | wc -l | tr -d ' ')"

if command -v rg >/dev/null 2>&1; then
  tests=$(rg -c '#\[test\]' --glob '*.rs' -N 2>/dev/null | awk -F: '{s+=$2} END {print s+0}')
else
  tests=$(grep -R --include='*.rs' -c '#\[test\]' src 2>/dev/null | awk -F: '{s+=$2} END {print s+0}')
fi
echo "tests:   $tests"

count_tests() {
  local p="$1"
  if [[ -d "$p" ]]; then
    if command -v rg >/dev/null 2>&1; then
      rg -c '#\[test\]' --glob '*.rs' -N "$p" 2>/dev/null | awk -F: '{s+=$2} END {print s+0}'
    else
      grep -R --include='*.rs' -c '#\[test\]' "$p" 2>/dev/null | awk -F: '{s+=$2} END {print s+0}'
    fi
  else
    echo 0
  fi
}

echo "tests_worker: $(count_tests src/worker)"
echo "tests_tui:    $(count_tests src/tui)"
echo "tests_rlm:    $(count_tests src/rlm)"
echo "tests_beads:  $(count_tests src/beads)"
echo "tests_loop:   $(count_tests src/agent)"

echo "--- modules (rs file counts) ---"
find src -name '*.rs' | sed 's|/[^/]*$||' | sort | uniq -c | sort -nr | head -20

echo "--- screenshots ---"
if git ls-files '*.png' '*.gif' '*.jpg' '*.webp' 2>/dev/null | grep -q .; then
  git ls-files '*.png' '*.gif' '*.jpg' '*.webp'
else
  echo "(none tracked)"
fi

echo "--- ci rlm-demo ---"
if [[ -f .github/workflows/ci.yml ]]; then
  grep -n 'continue-on-error\|rlm-demo\|eval-agent' .github/workflows/ci.yml || true
fi

echo "--- github ---"
if command -v gh >/dev/null 2>&1; then
  gh repo view --json name,url,stargazerCount,forkCount,isPrivate,updatedAt 2>/dev/null || echo "gh repo view failed"
  echo "releases:"
  gh release list --limit 5 2>/dev/null || echo "(none / gh failed)"
else
  echo "gh not installed"
fi

echo "=== end snapshot ==="
