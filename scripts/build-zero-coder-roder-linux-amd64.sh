#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
profile_path="${repo_root}/dist/zero-coder-edits-profile.toml"
dist_dir="${repo_root}/dist/zero-coder-roder"
artifact_dir="${repo_root}/dist/artifacts"
target="${RODER_ZERO_CODER_TARGET:-x86_64-unknown-linux-gnu}"
binary_name="zero-coder-roder"
toolchain="${RODER_RUSTUP_TOOLCHAIN:-stable}"
cargo_cmd=(rustup run "${toolchain}" cargo)
rustc_path="$(rustup which --toolchain "${toolchain}" rustc)"

if ! command -v cargo-zigbuild >/dev/null 2>&1; then
  echo "cargo-zigbuild is required for cross-compiling ${target}" >&2
  echo "install with: cargo install cargo-zigbuild --locked" >&2
  exit 1
fi

if ! command -v zig >/dev/null 2>&1; then
  echo "zig is required for cargo-zigbuild" >&2
  exit 1
fi

cd "${repo_root}"
rustup target add --toolchain "${toolchain}" "${target}"

mkdir -p "${artifact_dir}"
"${cargo_cmd[@]}" run -q -p roder-configure -- profile show zero-coder-edits > "${profile_path}"
"${cargo_cmd[@]}" run -q -p roder-configure -- validate "${profile_path}"
rm -rf "${dist_dir}"
"${cargo_cmd[@]}" run -q -p roder-configure -- generate --profile "${profile_path}" --out "${dist_dir}"

(
  cd "${dist_dir}"
  RUSTC="${rustc_path}" "${cargo_cmd[@]}" zigbuild --release --target "${target}"
)

cp "${dist_dir}/target/${target}/release/${binary_name}" \
  "${artifact_dir}/${binary_name}-${target}"
cp "${dist_dir}/config.toml" \
  "${artifact_dir}/${binary_name}.config.toml"

echo "${artifact_dir}/${binary_name}-${target}"
echo "${artifact_dir}/${binary_name}.config.toml"
