#!/usr/bin/env bash
set -euo pipefail

# Idempotent dependency install for Cursor Cloud Agents.
# Runs from the repository root (see .cursor/environment.json).

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

ensure_rust() {
  if ! command -v rustc >/dev/null 2>&1; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --default-toolchain 1.95.0 --profile minimal
    # shellcheck disable=SC1091
    source "${HOME}/.cargo/env"
  fi
  rustup toolchain install 1.95.0 --profile minimal
  rustup default 1.95.0
}

ensure_nextest() {
  if command -v cargo-nextest >/dev/null 2>&1; then
    return 0
  fi
  cargo install cargo-nextest --locked
}

ensure_uv() {
  if command -v uv >/dev/null 2>&1; then
    return 0
  fi
  curl -LsSf https://astral.sh/uv/install.sh | sh
  export PATH="${HOME}/.local/bin:${PATH}"
}

echo "==> Ensuring Rust 1.95.0"
ensure_rust
rustc --version
cargo --version

echo "==> Fetching Cargo dependencies"
cargo fetch

echo "==> Installing cargo-nextest (optional test runner)"
ensure_nextest || echo "warning: cargo-nextest install failed; make test will fall back to cargo test"

echo "==> Building roder CLI"
make build

echo "==> Installing Python e2e dependencies (tuiwright from PyPI)"
ensure_uv
export PATH="${HOME}/.local/bin:${PATH}"
uv python install 3.12
(cd e2e && uv sync --group dev)

echo "==> Cloud agent setup complete"
