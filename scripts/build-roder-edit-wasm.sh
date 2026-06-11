#!/usr/bin/env bash
# Builds the roder-edit-wasm crate for wasm32-unknown-unknown and generates
# Node bindings into packages/edit-tools/wasm/ via wasm-bindgen.
#
# Requirements:
#   - a rustup toolchain with the wasm32-unknown-unknown std component
#     (`rustup target add wasm32-unknown-unknown`)
#   - wasm-bindgen-cli matching the workspace's wasm-bindgen crate version
#     (`cargo install wasm-bindgen-cli --version <lockfile version>`)
#
# Note: when the active `cargo`/`rustc` come from Homebrew (as in the local
# mise setup), the rustup toolchain must be used explicitly because Homebrew
# rust cannot install the wasm32 std component.
set -euo pipefail

cd "$(dirname "$0")/.."

OUT_DIR="packages/edit-tools/wasm"
TARGET_DIR_WASM="target/wasm32-unknown-unknown/release/roder_edit_wasm.wasm"

if ! command -v wasm-bindgen >/dev/null; then
  echo "wasm-bindgen-cli is not installed; run: cargo install wasm-bindgen-cli" >&2
  exit 2
fi

build() {
  cargo build -p roder-edit-wasm --target wasm32-unknown-unknown --release
}

if ! build 2>/dev/null; then
  # Homebrew rust lacks wasm32 std; retry through the rustup toolchain.
  RUSTUP_RUSTC="$(rustup which rustc --toolchain stable 2>/dev/null || true)"
  RUSTUP_CARGO="${RUSTUP_RUSTC%rustc}cargo"
  if [[ -z "$RUSTUP_RUSTC" || ! -x "$RUSTUP_CARGO" ]]; then
    echo "wasm32 build failed and no rustup stable toolchain is available" >&2
    exit 2
  fi
  echo "retrying wasm32 build with the rustup stable toolchain" >&2
  RUSTC="$RUSTUP_RUSTC" "$RUSTUP_CARGO" build -p roder-edit-wasm \
    --target wasm32-unknown-unknown --release
fi

mkdir -p "$OUT_DIR"
wasm-bindgen "$TARGET_DIR_WASM" --target nodejs --out-dir "$OUT_DIR"
# The nodejs bindgen output is CommonJS, but the package is "type": "module";
# scope the generated directory back to CJS so require() loads it.
cat > "$OUT_DIR/package.json" <<'JSON'
{ "type": "commonjs" }
JSON
echo "generated:"
ls -la "$OUT_DIR"
