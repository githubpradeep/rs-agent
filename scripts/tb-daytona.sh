#!/usr/bin/env bash
# Run rs-agent on Terminal-Bench 2.0 via Harbor, OpenRouter, and Daytona.
#
# Keys live in ~/.rs-agent/harbor.env (not in this repo):
#   OPENROUTER_API_KEY=...
#   DAYTONA_API_KEY=...
#
#   ./scripts/tb-daytona.sh --build          # linux/amd64 binary for the sandbox
#   ./scripts/tb-daytona.sh                  # one task, one at a time
#   ./scripts/tb-daytona.sh -l 3 -n 2        # extra flags go straight to harbor
#   TB_MODEL=deepseek/deepseek-v4.1-flash ./scripts/tb-daytona.sh -l 89 -n 8
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ENV_FILE="${HARBOR_ENV:-$HOME/.rs-agent/harbor.env}"
BIN="$ROOT/target/x86_64-unknown-linux-gnu/release/rs-agent"

build_linux() {
  local zig_dir="$HOME/.local/zig"
  local zig_bin="$zig_dir/zig"
  local czb="$HOME/.local/bin/cargo-zigbuild"
  mkdir -p "$HOME/.local/bin"
  if [[ ! -x "$zig_bin" ]]; then
    echo "Downloading zig 0.14.1..."
    local tmp
    tmp="$(mktemp -d)"
    curl -fsSL -o "$tmp/zig.tar.xz" \
      "https://ziglang.org/download/0.14.1/zig-aarch64-macos-0.14.1.tar.xz"
    tar -C "$tmp" -xf "$tmp/zig.tar.xz"
    rm -rf "$zig_dir"
    mv "$tmp"/zig-aarch64-macos-* "$zig_dir"
    rm -rf "$tmp"
  fi
  if [[ ! -x "$czb" ]]; then
    echo "Downloading cargo-zigbuild 0.23.4..."
    local tmp
    tmp="$(mktemp -d)"
    curl -fsSL -o "$tmp/czb.tar.xz" \
      "https://github.com/rust-cross/cargo-zigbuild/releases/download/v0.23.4/cargo-zigbuild-aarch64-apple-darwin.tar.xz"
    tar -C "$tmp" -xf "$tmp/czb.tar.xz"
    install -m 755 "$tmp"/cargo-zigbuild-*/cargo-zigbuild "$czb"
    rm -rf "$tmp"
  fi
  export PATH="$zig_dir:$HOME/.local/bin:$PATH"
  unset CARGO_TARGET_DIR
  echo "Building linux/amd64 rs-agent..."
  RUSTUP_USE_CURL=1 rustup target add x86_64-unknown-linux-gnu
  cargo zigbuild --release --target x86_64-unknown-linux-gnu
  file "$BIN"
}

if [[ "${1:-}" == "--build" ]]; then
  build_linux
  exit 0
fi

if [[ ! -f "$ENV_FILE" ]]; then
  echo "Missing $ENV_FILE" >&2
  exit 1
fi
if [[ ! -x "$BIN" ]]; then
  echo "Missing Linux binary: $BIN" >&2
  echo "Run: ./scripts/tb-daytona.sh --build" >&2
  exit 1
fi

set -a
# shellcheck disable=SC1090
source "$ENV_FILE"
set +a

if [[ -z "${OPENROUTER_API_KEY:-}" || -z "${DAYTONA_API_KEY:-}" ]]; then
  echo "$ENV_FILE must set OPENROUTER_API_KEY and DAYTONA_API_KEY" >&2
  exit 1
fi

export PYTHONUNBUFFERED=1
export PYTHONPATH="$ROOT${PYTHONPATH:+:$PYTHONPATH}"
cd "$ROOT"

# Harbor's Python trusts certifi, not the macOS keychain. On networks that
# intercept TLS, pip-system-certs makes that interpreter use the OS store.
HARBOR_PY="$HOME/.local/share/uv/tools/harbor/bin/python"
if [[ -x "$HARBOR_PY" ]] && ! "$HARBOR_PY" -c "import pip_system_certs" >/dev/null 2>&1; then
  UV_SYSTEM_CERTS=1 uv pip install --python "$HARBOR_PY" pip-system-certs
fi

if [[ $# -eq 0 ]]; then
  set -- -l 1 -n 1
fi

exec harbor run \
  -d terminal-bench@2.0 \
  -a harbor_agents.rs_agent:RsAgent \
  -m "${TB_MODEL:-deepseek/deepseek-v4.1-flash}" \
  -e daytona \
  "$@"
