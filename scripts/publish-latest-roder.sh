#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

targets="${RODER_PUBLISH_TARGETS:-x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu}"
dist_dir="${RODER_PUBLISH_DIST_DIR:-dist/latest}"
remote_dist_dir="${RODER_REMOTE_DIST_DIR:-dist/remote-roder}"
r2_account_id="${R2_ACCOUNT_ID:-769befa385792ae6e7ca7136b7010256}"
r2_bucket="${R2_BUCKET:-roder-downloads}"
r2_public_base_url="${R2_PUBLIC_BASE_URL:-https://dl.roder.sh}"
dry_run="${RODER_PUBLISH_DRY_RUN:-0}"

if command -v cargo-zigbuild >/dev/null 2>&1 || cargo zigbuild --version >/dev/null 2>&1; then
  cargo_build=(cargo zigbuild)
  if [[ -z "${RUSTC:-}" ]] && command -v rustup >/dev/null 2>&1; then
    export RUSTC="$(rustup which rustc)"
  fi
else
  cargo_build=(cargo build)
fi

checksum() {
  local file="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$file"
  else
    shasum -a 256 "$file"
  fi
}

write_checksum() {
  local file="$1"
  local name
  name="$(basename "$file")"
  (cd "$(dirname "$file")" && checksum "$name" > "$name.sha256")
}

package_binary() {
  local binary_path="$1"
  local binary_name="$2"
  local target="$3"
  local staging archive_base
  staging="$(mktemp -d)"
  archive_base="${binary_name}-${target}"
  mkdir -p "$staging/$archive_base"
  cp "$binary_path" "$staging/$archive_base/$binary_name"
  chmod 0755 "$staging/$archive_base/$binary_name"
  cat > "$staging/$archive_base/README.txt" <<README
${binary_name} ${target}

Install:
  install -m 0755 ${binary_name} /usr/local/bin/${binary_name}
README
  tar -C "$staging" -czf "$dist_dir/${archive_base}.tar.gz" "$archive_base"
  (
    cd "$staging"
    zip -qr "$repo_root/$dist_dir/${archive_base}.zip" "$archive_base"
  )
  rm -rf "$staging"
  write_checksum "$dist_dir/${archive_base}.tar.gz"
  write_checksum "$dist_dir/${archive_base}.zip"
}

mkdir -p "$dist_dir"
rm -f "$dist_dir"/roder-* "$dist_dir"/remote-roder-* "$dist_dir"/SHA256SUMS "$dist_dir"/manifest.json "$dist_dir"/install.sh

cargo run -p roder-configure -- profile show remote-app-server > dist/remote-app-server-profile.toml
rm -rf "$remote_dist_dir"
cargo run -p roder-configure -- generate \
  --profile dist/remote-app-server-profile.toml \
  --out "$remote_dist_dir"

for target in $targets; do
  echo "publish: building roder for $target"
  "${cargo_build[@]}" --release -p roder --bin roder --target "$target"

  echo "publish: building remote-roder for $target"
  (
    cd "$remote_dist_dir"
    "${cargo_build[@]}" --release --target "$target"
  )

  roder_dest="$dist_dir/roder-$target"
  remote_dest="$dist_dir/remote-roder-$target"
  cp "target/$target/release/roder" "$roder_dest"
  cp "$remote_dist_dir/target/$target/release/remote-roder" "$remote_dest"
  strip "$roder_dest" 2>/dev/null || true
  strip "$remote_dest" 2>/dev/null || true
  chmod 0755 "$roder_dest" "$remote_dest"

  write_checksum "$roder_dest"
  write_checksum "$remote_dest"
  package_binary "$roder_dest" roder "$target"
  package_binary "$remote_dest" remote-roder "$target"
done

cp scripts/install-roder.sh "$dist_dir/install.sh"
chmod 0644 "$dist_dir/install.sh"

(
  cd "$dist_dir"
  : > SHA256SUMS
  for artifact in roder-* remote-roder-*; do
    case "$artifact" in
      *.sha256) continue ;;
    esac
    [[ -e "$artifact" ]] || continue
    checksum "$artifact" >> SHA256SUMS
  done
)

R2_PUBLIC_BASE_URL="$r2_public_base_url" python3 - <<'PY'
import json
import os
import subprocess
from pathlib import Path

root = Path(os.environ.get("RODER_PUBLISH_DIST_DIR", "dist/latest"))
try:
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
except Exception:
    commit = "unknown"
artifacts = []
for path in sorted([*root.glob("roder-*"), *root.glob("remote-roder-*")]):
    if path.name.endswith(".sha256"):
        continue
    digest = (root / f"{path.name}.sha256").read_text().split()[0]
    prefix = "remote-roder-" if path.name.startswith("remote-roder-") else "roder-"
    distribution = "remote-app-server" if prefix == "remote-roder-" else "default"
    target = path.name.removeprefix(prefix)
    kind = "binary"
    if target.endswith(".tar.gz"):
        target = target.removesuffix(".tar.gz")
        kind = "tar.gz"
    elif target.endswith(".zip"):
        target = target.removesuffix(".zip")
        kind = "zip"
    artifacts.append({
        "name": path.name,
        "target": target,
        "distribution": distribution,
        "kind": kind,
        "url": f"{os.environ['R2_PUBLIC_BASE_URL']}/latest/{path.name}",
        "sha256": digest,
        "bytes": path.stat().st_size,
    })
manifest = {
    "version": "latest",
    "commit": commit,
    "artifacts": artifacts,
    "install": f"{os.environ['R2_PUBLIC_BASE_URL']}/install.sh",
    "install_latest": f"{os.environ['R2_PUBLIC_BASE_URL']}/latest/install.sh",
}
(root / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
PY

if [[ "$dry_run" == "1" ]]; then
  echo "publish: dry run complete; artifacts are in $dist_dir"
  exit 0
fi

if [[ -z "${CLOUDFLARE_API_TOKEN:-}" ]]; then
  echo "publish: CLOUDFLARE_API_TOKEN is required for R2 upload; set RODER_PUBLISH_DRY_RUN=1 to build only" >&2
  exit 1
fi
if ! command -v aws >/dev/null 2>&1; then
  echo "publish: aws CLI is required for R2 upload" >&2
  exit 1
fi
if ! command -v jq >/dev/null 2>&1; then
  echo "publish: jq is required for R2 credential derivation" >&2
  exit 1
fi

verify_json="$(curl -fsSL \
  -H "Authorization: Bearer ${CLOUDFLARE_API_TOKEN}" \
  "https://api.cloudflare.com/client/v4/accounts/${r2_account_id}/tokens/verify")"
access_key_id="$(jq -r '.result.id // empty' <<<"$verify_json")"
status="$(jq -r '.result.status // empty' <<<"$verify_json")"
if [[ -z "$access_key_id" || "$status" != "active" ]]; then
  echo "publish: Cloudflare account token verification failed" >&2
  exit 1
fi
if command -v sha256sum >/dev/null 2>&1; then
  secret_access_key="$(printf '%s' "$CLOUDFLARE_API_TOKEN" | sha256sum | awk '{print $1}')"
else
  secret_access_key="$(printf '%s' "$CLOUDFLARE_API_TOKEN" | shasum -a 256 | awk '{print $1}')"
fi
endpoint="https://${r2_account_id}.r2.cloudflarestorage.com"

for path in "$dist_dir"/*; do
  name="$(basename "$path")"
  case "$name" in
    install.sh) content_type="text/x-shellscript; charset=utf-8" ;;
    manifest.json) content_type="application/json; charset=utf-8" ;;
    SHA256SUMS|*.sha256) content_type="text/plain; charset=utf-8" ;;
    *.tar.gz|*.zip) content_type="application/octet-stream" ;;
    *) content_type="application/octet-stream" ;;
  esac
  AWS_ACCESS_KEY_ID="$access_key_id" \
  AWS_SECRET_ACCESS_KEY="$secret_access_key" \
  AWS_DEFAULT_REGION=auto \
    aws s3 cp "$path" "s3://${r2_bucket}/latest/${name}" \
      --endpoint-url "$endpoint" \
      --content-type "$content_type" \
      --cache-control "public, max-age=300" \
      --no-progress
  if [[ "$name" == "install.sh" ]]; then
    AWS_ACCESS_KEY_ID="$access_key_id" \
    AWS_SECRET_ACCESS_KEY="$secret_access_key" \
    AWS_DEFAULT_REGION=auto \
      aws s3 cp "$path" "s3://${r2_bucket}/install.sh" \
        --endpoint-url "$endpoint" \
        --content-type "$content_type" \
        --cache-control "public, max-age=300" \
        --no-progress
  fi
done

echo "publish: uploaded ${r2_public_base_url}/latest/manifest.json"
echo "publish: uploaded ${r2_public_base_url}/install.sh"
