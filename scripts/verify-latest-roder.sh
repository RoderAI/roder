#!/usr/bin/env bash
set -euo pipefail

base_url="${R2_PUBLIC_BASE_URL:-https://dl.roder.sh}"
targets="${RODER_PUBLISH_TARGETS:-x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu}"
attempts="${RODER_PUBLISH_VERIFY_ATTEMPTS:-1}"
delay_seconds="${RODER_PUBLISH_VERIFY_DELAY_SECONDS:-10}"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

run_check() {
  local manifest_url="${base_url%/}/latest/manifest.json"
  local manifest_path="$tmp_dir/manifest.json"
  local status
  status="$(curl -fsSL -o "$manifest_path" -w '%{http_code}' "$manifest_url" || true)"
  if [[ "$status" != "200" ]]; then
    echo "publish-verify: missing manifest: $manifest_url (HTTP ${status:-failed})" >&2
    return 1
  fi

  BASE_URL="${base_url%/}" TARGETS="$targets" MANIFEST_PATH="$manifest_path" python3 - <<'PY'
import json
import os
import sys
import urllib.request
from pathlib import Path

base_url = os.environ["BASE_URL"]
targets = os.environ["TARGETS"].split()
manifest = json.loads(Path(os.environ["MANIFEST_PATH"]).read_text())
artifacts = manifest.get("artifacts", [])

errors = []
by_name = {artifact.get("name"): artifact for artifact in artifacts}
for target in targets:
    name = f"remote-roder-{target}"
    artifact = by_name.get(name)
    if not artifact:
        errors.append(f"manifest missing {name}")
        continue
    if artifact.get("distribution") != "remote-app-server":
        errors.append(f"{name} distribution is {artifact.get('distribution')!r}")
    expected_url = f"{base_url}/latest/{name}"
    if artifact.get("url") != expected_url:
        errors.append(f"{name} url is {artifact.get('url')!r}, expected {expected_url!r}")
    if not artifact.get("sha256"):
        errors.append(f"{name} missing sha256")
    if not artifact.get("bytes"):
        errors.append(f"{name} missing byte size")

    for suffix in ("", ".sha256"):
        url = f"{base_url}/latest/{name}{suffix}"
        request = urllib.request.Request(url, method="HEAD")
        try:
            with urllib.request.urlopen(request, timeout=20) as response:
                if response.status != 200:
                    errors.append(f"{url} returned HTTP {response.status}")
        except Exception as exc:
            errors.append(f"{url} failed: {exc}")

if errors:
    for error in errors:
        print(f"publish-verify: {error}", file=sys.stderr)
    sys.exit(1)

print(
    "publish-verify: remote-roder artifacts present for "
    + ", ".join(targets)
)
PY
}

last_status=0
for attempt in $(seq 1 "$attempts"); do
  if run_check; then
    exit 0
  else
    last_status=$?
  fi
  if [[ "$attempt" != "$attempts" ]]; then
    echo "publish-verify: attempt $attempt/$attempts failed; retrying in ${delay_seconds}s" >&2
    sleep "$delay_seconds"
  fi
done

exit "$last_status"
