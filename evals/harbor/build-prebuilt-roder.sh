#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/../.."

mkdir -p evals/harbor/artifacts

image="${RODER_HARBOR_BUILDER_IMAGE:-rust:1-bullseye}"
platform="${RODER_HARBOR_BUILDER_PLATFORM:-linux/amd64}"
target_dir="${RODER_HARBOR_BUILDER_TARGET_DIR:-/tmp/roder-harbor-target}"
target_volume="${RODER_HARBOR_BUILDER_TARGET_VOLUME:-roder-harbor-target-bullseye-amd64}"

docker run --rm \
  --platform "$platform" \
  -e CARGO_TARGET_DIR="$target_dir" \
  -v "$PWD:/workspace" \
  -v roder-harbor-cargo-registry:/usr/local/cargo/registry \
  -v roder-harbor-cargo-git:/usr/local/cargo/git \
  -v "$target_volume":"$target_dir" \
  -w /workspace \
  "$image" \
  bash -c 'export PATH=/usr/local/cargo/bin:$PATH && apt-get update && apt-get install -y --no-install-recommends build-essential ca-certificates clang cmake lld make perl pkg-config && cargo build -p roder-cli --bin roder && cp "$CARGO_TARGET_DIR/debug/roder" /workspace/evals/harbor/artifacts/roder-linux-amd64 && file /workspace/evals/harbor/artifacts/roder-linux-amd64 && (ldd /workspace/evals/harbor/artifacts/roder-linux-amd64 || true)'

chmod +x evals/harbor/artifacts/roder-linux-amd64
echo "Wrote evals/harbor/artifacts/roder-linux-amd64"
