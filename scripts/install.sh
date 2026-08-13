#!/usr/bin/env bash
set -euo pipefail

REPO="githubpradeep/rs-agent"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"
VERSION="${VERSION:-}"

os="$(uname -s)"
arch="$(uname -m)"

case "$os" in
  Linux)  platform="linux" ;;
  Darwin) platform="macos" ;;
  *)
    echo "error: unsupported OS '$os' (supported: Linux, Darwin)" >&2
    exit 1
    ;;
esac

case "$arch" in
  x86_64|amd64) arch="x86_64" ;;
  aarch64|arm64) arch="aarch64" ;;
  *)
    echo "error: unsupported architecture '$arch' (supported: x86_64, aarch64)" >&2
    exit 1
    ;;
esac

if [[ "$platform" == "macos" && "$arch" == "x86_64" ]]; then
  echo "error: only aarch64 (Apple Silicon) macOS binaries are published." >&2
  echo "       Build from source: cargo install --path . --locked" >&2
  exit 1
fi

artifact="rs-agent-${platform}-${arch}"

if [[ -n "$VERSION" ]]; then
  # Accept either "v0.1.0" or "0.1.0"
  tag="$VERSION"
  [[ "$tag" == v* ]] || tag="v${tag}"
  url="https://github.com/${REPO}/releases/download/${tag}/${artifact}"
else
  url="https://github.com/${REPO}/releases/latest/download/${artifact}"
fi

mkdir -p "$INSTALL_DIR"
dest="${INSTALL_DIR}/rs-agent"

echo "Downloading ${artifact}..."
if ! curl -fsSL "$url" -o "$dest"; then
  echo "error: failed to download ${url}" >&2
  echo "       No GitHub release for this tag/platform yet (need a v* tag)." >&2
  echo "       Build from source:" >&2
  echo "         git clone https://github.com/${REPO}.git && cd rs-agent && cargo install --path . --locked" >&2
  exit 1
fi

chmod +x "$dest"
echo "Installed rs-agent to ${dest}"

case ":${PATH}:" in
  *:"${INSTALL_DIR}":*) ;;
  *)
    echo
    echo "Note: ${INSTALL_DIR} is not on your PATH."
    echo "Add it with:"
    echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
    ;;
esac
