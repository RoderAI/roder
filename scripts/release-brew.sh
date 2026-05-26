#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: VERSION=0.1.0 make release-brew
   or: scripts/release-brew.sh 0.1.0

Creates a local Homebrew source release for the Roder Rust CLI:
  - runs cargo test --workspace unless RUN_TESTS=0
  - creates/uses git tag v$VERSION unless CREATE_TAG=0
  - writes dist/roder-v$VERSION.tar.gz from the tagged source
  - updates Formula/roder.rb to build roder from source

Environment:
  MODE=local|git       local uses the dist tarball; git uses REPO_URL.git tag+revision
  REPO_URL=URL         default: https://github.com/PandelisZ/gode
  FORMULA=PATH         default: Formula/roder.rb
  RUN_TESTS=0          skip tests
  ALLOW_DIRTY=1        allow releasing from a dirty working tree (for local testing only)
  CREATE_TAG=0         do not create a local git tag
  COMMIT=1             commit the formula update (requires MODE=git)
  PUSH=1               push the tag, and the formula commit when COMMIT=1, to REMOTE
  PUBLISH=1            shorthand for MODE=git COMMIT=1 PUSH=1
  REMOTE=name          default: origin
USAGE
}

version="${VERSION:-${1:-}}"
if [[ "$version" == "-h" || "$version" == "--help" ]]; then
  usage
  exit 0
fi
if [[ -z "$version" ]]; then
  usage >&2
  exit 2
fi

version="${version#v}"
tag="v${version}"
mode="${MODE:-local}"
repo_url="${REPO_URL:-https://github.com/PandelisZ/gode}"
formula="${FORMULA:-Formula/roder.rb}"
run_tests="${RUN_TESTS:-1}"
create_tag="${CREATE_TAG:-1}"
commit_formula="${COMMIT:-0}"
push_tag="${PUSH:-0}"
remote="${REMOTE:-origin}"

if [[ "${PUBLISH:-0}" == "1" ]]; then
  mode="git"
  commit_formula="1"
  push_tag="1"
fi

case "$mode" in
  local|git) ;;
  *) echo "release-brew: MODE must be 'local' or 'git'" >&2; exit 2 ;;
esac

if [[ "$commit_formula" == "1" && "$mode" != "git" ]]; then
  echo "release-brew: COMMIT=1 requires MODE=git so the committed formula does not point at a local dist file" >&2
  exit 2
fi

if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-.][0-9A-Za-z.-]+)?$ ]]; then
  echo "release-brew: VERSION must look like 0.1.0 or v0.1.0" >&2
  exit 2
fi

root="$(git rev-parse --show-toplevel)"
cd "$root"

workspace_version="$(python3 - <<'PY'
import re

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
PY
)"

if [[ "$workspace_version" != "$version" ]]; then
  cat >&2 <<EOF
release-brew: VERSION=$version does not match [workspace.package].version=$workspace_version.
Bump the root Cargo.toml workspace version first, then rerun this release.
EOF
  exit 1
fi

if [[ -n "$(git status --porcelain)" ]]; then
  if [[ "${ALLOW_DIRTY:-0}" == "1" ]]; then
    echo "release-brew: warning: working tree is dirty; archive/formula may not include uncommitted changes" >&2
  else
  cat >&2 <<EOF
release-brew: working tree is dirty.
Commit or stash changes before cutting a release so the tag/archive matches the formula.
EOF
  exit 1
  fi
fi

if [[ "$run_tests" == "1" ]]; then
  cargo test --workspace
fi

if [[ "$create_tag" == "1" ]]; then
  if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
    echo "release-brew: using existing tag $tag"
  else
    git tag -a "$tag" -m "roder $tag"
    echo "release-brew: created tag $tag"
  fi
  source_ref="$tag"
else
  source_ref="HEAD"
fi

mkdir -p dist Formula
revision="$(git rev-parse "$source_ref^{commit}")"
archive="dist/roder-$tag.tar.gz"

git archive --format=tar --prefix="roder-$tag/" "$source_ref" | gzip -n > "$archive"
sha256="$(shasum -a 256 "$archive" | awk '{print $1}')"

if [[ "$mode" == "local" ]]; then
  url_block=$(cat <<EOF
  url "file://#{File.expand_path("../dist/roder-$tag.tar.gz", __dir__)}"
  sha256 "$sha256"
EOF
)
else
  url_block=$(cat <<EOF
  url "$repo_url.git",
      tag: "$tag",
      revision: "$revision"
EOF
)
fi

cat > "$formula" <<EOF
class Roder < Formula
  desc "Rust-native TUI coding agent and event-driven agent harness"
  homepage "$repo_url"
  version "$version"
$url_block
  head "$repo_url.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args(path: "crates/roder-cli")
  end

  test do
    assert_match "codex:", shell_output("#{bin}/roder auth status")
  end
end
EOF

cat <<EOF
release-brew: wrote $formula
release-brew: wrote $archive
release-brew: sha256 $sha256
release-brew: source revision $revision

Install locally with:
  brew install --build-from-source ./$formula

Reinstall/upgrade locally with:
  brew reinstall --build-from-source ./$formula
EOF

if [[ "$commit_formula" == "1" ]]; then
  git add "$formula"
  if git diff --cached --quiet -- "$formula"; then
    echo "release-brew: no formula changes to commit"
  else
    git commit -m "brew: update roder to $tag" -- "$formula"
  fi
fi

if [[ "$push_tag" == "1" ]]; then
  git push "$remote" "$tag"
  if [[ "$commit_formula" == "1" ]]; then
    branch="$(git branch --show-current)"
    if [[ -z "$branch" ]]; then
      echo "release-brew: not on a branch; formula commit was not pushed" >&2
    else
      git push "$remote" "$branch"
    fi
  fi
  cat <<EOF

release-brew: pushed tag $tag to $remote.
EOF
fi
