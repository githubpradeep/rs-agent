#!/usr/bin/env bash
# Real-world agent eval harness — isolated workspaces + assertable checks.
#
# Usage:
#   ./scripts/eval-agent.sh
#   ./scripts/eval-agent.sh --provider anthropic --model claude-sonnet-4-20250514
#   ./scripts/eval-agent.sh --only write_hello,fix_bug
#   ./scripts/eval-agent.sh --with-network
#   ./scripts/eval-agent.sh --list
#
# Requires: target/release/rs-agent (or RS_AGENT_BIN), python3, provider credentials.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TASKS_DIR="$ROOT/evals/tasks"
BIN="${RS_AGENT_BIN:-$ROOT/target/release/rs-agent}"
PROVIDER="${PROVIDER:-}"
MODEL="${MODEL:-}"
TIMEOUT="${TIMEOUT:-600}"
ONLY=""
WITH_NETWORK=0
LIST=0

CORE_TASKS=(write_hello fix_bug find_secret rlm_needle rlm_escalate edit_whitespace_drift post_edit_type_error bash_long_output parallel_same_file)
NETWORK_TASKS=(webfetch_title)

while [[ $# -gt 0 ]]; do
  case "$1" in
    --provider) PROVIDER="$2"; shift 2 ;;
    --model) MODEL="$2"; shift 2 ;;
    --timeout) TIMEOUT="$2"; shift 2 ;;
    --bin) BIN="$2"; shift 2 ;;
    --only) ONLY="$2"; shift 2 ;;
    --with-network) WITH_NETWORK=1; shift ;;
    --list) LIST=1; shift ;;
    -h|--help)
      sed -n '2,14p' "$0"
      exit 0
      ;;
    *)
      echo "unknown arg: $1" >&2
      exit 1
      ;;
  esac
done

if [[ ! -x "$BIN" ]]; then
  echo "==> Building release binary…"
  (cd "$ROOT" && cargo build --release)
  BIN="$ROOT/target/release/rs-agent"
fi

if [[ "$LIST" -eq 1 ]]; then
  echo "Core tasks: ${CORE_TASKS[*]}"
  echo "Network tasks (--with-network): ${NETWORK_TASKS[*]}"
  exit 0
fi

TASKS=("${CORE_TASKS[@]}")
if [[ "$WITH_NETWORK" -eq 1 ]]; then
  TASKS+=("${NETWORK_TASKS[@]}")
fi

if [[ -n "$ONLY" ]]; then
  IFS=',' read -r -a TASKS <<< "$ONLY"
fi

ARGS=(-a --timeout "$TIMEOUT" --no-context-files)
if [[ -n "$PROVIDER" ]]; then
  ARGS+=(--provider "$PROVIDER")
fi
if [[ -n "$MODEL" ]]; then
  ARGS+=(--model "$MODEL")
fi

PASS=0
FAIL=0
SKIP=0
RESULTS=()

run_task() {
  local id="$1"
  local task_dir="$TASKS_DIR/$id"
  if [[ ! -d "$task_dir" ]]; then
    echo "FAIL  $id  (missing $task_dir)"
    RESULTS+=("FAIL $id missing-task-dir")
    FAIL=$((FAIL + 1))
    return
  fi
  if [[ ! -f "$task_dir/prompt.txt" || ! -f "$task_dir/check.sh" ]]; then
    echo "FAIL  $id  (need prompt.txt + check.sh)"
    RESULTS+=("FAIL $id incomplete")
    FAIL=$((FAIL + 1))
    return
  fi

  local work
  work="$(mktemp -d "${TMPDIR:-/tmp}/rs-agent-eval-${id}.XXXXXX")"
  local log="$work/agent.log"

  # Seed workspace
  if [[ -d "$task_dir/fixture" ]]; then
    cp -R "$task_dir/fixture/." "$work/"
  fi
  if [[ -f "$task_dir/setup.sh" ]]; then
    (cd "$work" && bash "$task_dir/setup.sh") >>"$log" 2>&1 || {
      echo "FAIL  $id  (setup.sh failed)"
      RESULTS+=("FAIL $id setup")
      FAIL=$((FAIL + 1))
      return
    }
  fi

  local prompt
  prompt="$(cat "$task_dir/prompt.txt")"

  echo "==> $id  (cwd=$work)"
  set +e
  (
    cd "$work"
    "$BIN" "${ARGS[@]}" -p "$prompt"
  ) >"$log" 2>&1
  local agent_status=$?
  set -e

  export EVAL_AGENT_LOG="$log"
  set +e
  (cd "$work" && bash "$task_dir/check.sh") >>"$log" 2>&1
  local check_status=$?
  set -e

  if [[ $check_status -eq 0 ]]; then
    echo "PASS  $id"
    RESULTS+=("PASS $id")
    PASS=$((PASS + 1))
  else
    echo "FAIL  $id  (agent_exit=$agent_status check_exit=$check_status)"
    echo "      log: $log"
    # Show a short tail for debugging
    tail -n 40 "$log" | sed 's/^/      | /'
    RESULTS+=("FAIL $id")
    FAIL=$((FAIL + 1))
  fi
}

echo "rs-agent evals"
echo "  bin: $BIN"
echo "  args: ${ARGS[*]}"
echo "  tasks: ${TASKS[*]}"
echo

for id in "${TASKS[@]}"; do
  id="$(echo "$id" | tr -d '[:space:]')"
  [[ -z "$id" ]] && continue
  run_task "$id"
done

echo
echo "==== summary ===="
for r in "${RESULTS[@]}"; do
  echo "  $r"
done
echo "pass=$PASS fail=$FAIL skip=$SKIP"

if [[ "$FAIL" -gt 0 ]]; then
  exit 1
fi
exit 0
