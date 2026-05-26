#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: scripts/check-version-bump.sh [BASE_REF]

Fails when release-affecting Cargo workspace changes are present but the root
[workspace.package].version in Cargo.toml was not increased.

BASE_REF may be any git revision. If omitted, the script uses BASE_REF from the
environment, then GITHUB_BASE_REF, then origin/master, then origin/main.
USAGE
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

root="$(git rev-parse --show-toplevel)"
cd "$root"

requested_base="${1:-${BASE_REF:-}}"

resolve_base() {
  local candidate="$1"

  if [[ -n "$candidate" && ! "$candidate" =~ ^0+$ ]]; then
    git rev-parse --verify "$candidate^{commit}" 2>/dev/null && return 0
  fi

  if [[ -n "${GITHUB_BASE_REF:-}" ]]; then
    git rev-parse --verify "origin/${GITHUB_BASE_REF}^{commit}" 2>/dev/null && return 0
  fi

  git rev-parse --verify "origin/master^{commit}" 2>/dev/null && return 0
  git rev-parse --verify "origin/main^{commit}" 2>/dev/null && return 0
  git rev-parse --verify "HEAD~1^{commit}" 2>/dev/null && return 0
}

base_commit="$(resolve_base "$requested_base" || true)"
if [[ -z "$base_commit" ]]; then
  echo "version-bump: could not resolve a base commit" >&2
  usage >&2
  exit 2
fi

release_affecting_count=0
while IFS= read -r file; do
  case "$file" in
    Cargo.toml|Cargo.lock|crates/*)
      release_affecting_count=$((release_affecting_count + 1))
      ;;
  esac
done < <(
  {
    git diff --name-only "$base_commit"...HEAD
    git diff --name-only
    git diff --cached --name-only
    git ls-files --others --exclude-standard
  } | sort -u
)

if (( release_affecting_count == 0 )); then
  echo "version-bump: no release-affecting Cargo workspace changes"
  exit 0
fi

old_version="$(
  git show "$base_commit:Cargo.toml" | python3 -c '
import re
import sys

in_workspace_package = False
for raw_line in sys.stdin:
    line = raw_line.strip()
    if line == "[workspace.package]":
        in_workspace_package = True
        continue
    if line.startswith("[") and line.endswith("]"):
        in_workspace_package = False
    if in_workspace_package:
        match = re.match(r"version\s*=\s*\"([^\"]+)\"", line)
        if match:
            print(match.group(1))
            raise SystemExit(0)
raise SystemExit("workspace package version not found")
'
)"

new_version="$(
  python3 -c '
import re
import sys

in_workspace_package = False
with open("Cargo.toml", encoding="utf-8") as manifest:
    for raw_line in manifest:
        line = raw_line.strip()
        if line == "[workspace.package]":
            in_workspace_package = True
            continue
        if line.startswith("[") and line.endswith("]"):
            in_workspace_package = False
        if in_workspace_package:
            match = re.match(r"version\s*=\s*\"([^\"]+)\"", line)
            if match:
                print(match.group(1))
                raise SystemExit(0)
raise SystemExit("workspace package version not found")
'
)"

python3 - "$old_version" "$new_version" <<'PY'
import re
import sys


def parse_semver(value: str):
    match = re.fullmatch(
        r"([0-9]+)\.([0-9]+)\.([0-9]+)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?",
        value,
    )
    if not match:
        raise SystemExit(f"version-bump: invalid semver version: {value}")
    major, minor, patch, prerelease = match.groups()
    return (int(major), int(minor), int(patch), parse_prerelease(prerelease))


def parse_prerelease(value):
    if value is None:
        return None
    parsed = []
    for ident in value.split("."):
        if ident.isdigit():
            parsed.append((0, int(ident)))
        else:
            parsed.append((1, ident))
    return parsed


def compare_prerelease(left, right):
    if left is None and right is None:
        return 0
    if left is None:
        return 1
    if right is None:
        return -1
    for left_ident, right_ident in zip(left, right):
        if left_ident == right_ident:
            continue
        return 1 if left_ident > right_ident else -1
    if len(left) == len(right):
        return 0
    return 1 if len(left) > len(right) else -1


old_version, new_version = sys.argv[1], sys.argv[2]
old = parse_semver(old_version)
new = parse_semver(new_version)

if new[:3] != old[:3]:
    is_greater = new[:3] > old[:3]
else:
    is_greater = compare_prerelease(new[3], old[3]) > 0

if not is_greater:
    print(
        f"version-bump: release-affecting changes require "
        f"[workspace.package].version to increase ({old_version} -> {new_version})",
        file=sys.stderr,
    )
    raise SystemExit(1)
PY

echo "version-bump: workspace version increased $old_version -> $new_version"
