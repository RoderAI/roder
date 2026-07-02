#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/../.."

mkdir -p evals/harbor/artifacts

image="${RODER_HARBOR_BUILDER_IMAGE:-rust:1-bullseye}"
platforms="${RODER_HARBOR_BUILDER_PLATFORMS:-linux/amd64 linux/arm64}"

for platform in $platforms; do
  case "$platform" in
    linux/amd64)
      arch=amd64
      ;;
    linux/arm64)
      arch=arm64
      ;;
    *)
      echo "Unsupported RODER_HARBOR_BUILDER_PLATFORMS entry: $platform" >&2
      exit 1
      ;;
  esac

  target_dir="${RODER_HARBOR_BUILDER_TARGET_DIR:-/tmp/roder-harbor-target-$arch}"
  target_volume="${RODER_HARBOR_BUILDER_TARGET_VOLUME:-roder-harbor-target-bullseye-$arch}"
  output="/workspace/evals/harbor/artifacts/roder-linux-$arch"

  docker run --rm \
    --platform "$platform" \
    -e CARGO_TARGET_DIR="$target_dir" \
    -v "$PWD:/workspace" \
    -v "roder-harbor-cargo-registry-$arch":/usr/local/cargo/registry \
    -v "roder-harbor-cargo-git-$arch":/usr/local/cargo/git \
    -v "$target_volume":"$target_dir" \
    -w /workspace \
    "$image" \
    bash -c 'export PATH=/usr/local/cargo/bin:$PATH && apt-get update && apt-get install -y --no-install-recommends build-essential ca-certificates clang cmake lld make perl pkg-config && cargo build -p roder --bin roder && cp "$CARGO_TARGET_DIR/debug/roder" "$0" && file "$0" && (ldd "$0" || true)' "$output"

  chmod +x "evals/harbor/artifacts/roder-linux-$arch"
  echo "Wrote evals/harbor/artifacts/roder-linux-$arch"
done
