#!/usr/bin/env bash
set -euo pipefail

# Update the Roder Homebrew tap so `brew install RoderAI/tap/roder` always
# tracks the latest released `roder` CLI version.
#
# What it does:
#   - resolves the `roder` CLI version (VERSION env, else crates/roder-cli/Cargo.toml)
#   - resolves the immutable GitHub source tag tarball and its sha256
#   - regenerates the tap's Formula/roder.rb for that version
#   - commits and pushes it to the tap repository (when a token is provided)
#
# It is invoked from .github/workflows/release.yml after `knope release`
# tags `roder/v<version>`, and is safe to run manually:
#
#   VERSION=0.1.5 HOMEBREW_TAP_TOKEN=ghp_... scripts/update-homebrew-tap.sh
#
# Without HOMEBREW_TAP_TOKEN it runs in dry-run mode and only writes the
# rendered formula under dist/ so the result can be inspected locally.
#
# Environment:
#   VERSION                  roder version (default: crates/roder-cli/Cargo.toml)
#   SOURCE_REPO              GitHub source repo, default: RoderAI/roder
#   TAP_REPO                 Homebrew tap repo, default: RoderAI/homebrew-tap
#   TAG                      source tag, default: roder/v$VERSION
#   FORMULA_PATH             formula path in the tap, default: Formula/roder.rb
#   HOMEBREW_TAP_TOKEN       token with push access to TAP_REPO (else dry-run)
#   OUTPUT_DIR               dry-run output dir, default: dist
#   RODER_TAP_DOWNLOAD_ATTEMPTS  tag-tarball fetch attempts, default: 12
#   RODER_TAP_DOWNLOAD_DELAY_SECONDS  delay between attempts, default: 10
#   RODER_TAP_REQUIRE_TAG    1 (default) fails when the tag is missing; 0 skips

usage() {
  sed -n '3,40p' "$0" | sed 's/^# \{0,1\}//'
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

version="${VERSION:-}"
if [[ -z "$version" ]]; then
  version="$(grep -m1 -E '^version[[:space:]]*=' "$repo_root/crates/roder-cli/Cargo.toml" \
    | sed -E 's/.*"([^"]+)".*/\1/')"
fi
version="${version#v}"
if [[ -z "$version" ]]; then
  echo "update-homebrew-tap: could not determine the roder version" >&2
  exit 2
fi

source_repo="${SOURCE_REPO:-RoderAI/roder}"
tap_repo="${TAP_REPO:-RoderAI/homebrew-tap}"
tag="${TAG:-roder/v${version}}"
formula_path="${FORMULA_PATH:-Formula/roder.rb}"
homepage="https://github.com/${source_repo}"
tarball_url="https://github.com/${source_repo}/archive/refs/tags/${tag}.tar.gz"

work="$(mktemp -d "${TMPDIR:-/tmp}/roder-tap.XXXXXX")"
cleanup() {
  rm -rf "$work"
}
trap cleanup EXIT

sha256_of() {
  local file="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$file" | awk '{print $1}'
  else
    shasum -a 256 "$file" | awk '{print $1}'
  fi
}

tarball="$work/roder-src.tar.gz"
attempts="${RODER_TAP_DOWNLOAD_ATTEMPTS:-12}"
delay="${RODER_TAP_DOWNLOAD_DELAY_SECONDS:-10}"
http_status=""
for ((attempt = 1; attempt <= attempts; attempt++)); do
  http_status="$(curl -sSL --connect-timeout 20 -w '%{http_code}' -o "$tarball" "$tarball_url" || echo "000")"
  if [[ "$http_status" == "200" ]]; then
    break
  fi
  echo "update-homebrew-tap: attempt ${attempt}/${attempts} for ${tag} returned HTTP ${http_status}; retrying in ${delay}s" >&2
  sleep "$delay"
done

if [[ "$http_status" != "200" ]]; then
  echo "update-homebrew-tap: source tag tarball not available (${tarball_url}, last HTTP ${http_status})." >&2
  if [[ "${RODER_TAP_REQUIRE_TAG:-1}" == "1" ]]; then
    exit 1
  fi
  echo "update-homebrew-tap: RODER_TAP_REQUIRE_TAG=0, skipping." >&2
  exit 0
fi

sha256="$(sha256_of "$tarball")"

render_formula() {
  cat <<EOF
class Roder < Formula
  desc "Rust-native TUI coding agent and event-driven agent harness"
  homepage "${homepage}"
  url "${tarball_url}"
  version "${version}"
  sha256 "${sha256}"
  head "https://github.com/${source_repo}.git", branch: "master"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args(path: "crates/roder-cli")
  end

  test do
    assert_match "codex:", shell_output("#{bin}/roder auth status")
  end
end
EOF
}

token="${HOMEBREW_TAP_TOKEN:-}"

if [[ -z "$token" ]]; then
  out_dir="${OUTPUT_DIR:-$repo_root/dist}"
  target="${out_dir}/${formula_path}"
  mkdir -p "$(dirname "$target")"
  render_formula >"$target"
  echo "update-homebrew-tap: HOMEBREW_TAP_TOKEN not set; wrote ${target} (dry-run, no push)."
  echo "update-homebrew-tap: version=${version} sha256=${sha256}"
  exit 0
fi

git_user_name="${GIT_AUTHOR_NAME:-github-actions[bot]}"
git_user_email="${GIT_AUTHOR_EMAIL:-github-actions[bot]@users.noreply.github.com}"

# Keep the token out of process args and remote URLs: stash it in a temporary
# credential store that is removed on exit.
credentials="$work/.git-credentials"
umask 077
printf 'https://x-access-token:%s@github.com\n' "$token" >"$credentials"

clone_dir="$work/tap"
git -c "credential.helper=store --file=${credentials}" \
  clone --depth 1 "https://github.com/${tap_repo}.git" "$clone_dir"

cd "$clone_dir"
git config user.name "$git_user_name"
git config user.email "$git_user_email"
git config "credential.helper" "store --file=${credentials}"

mkdir -p "$(dirname "$formula_path")"
render_formula >"$formula_path"

if git diff --quiet -- "$formula_path"; then
  echo "update-homebrew-tap: ${tap_repo} already at roder ${version}; nothing to update."
  exit 0
fi

git add "$formula_path"
git commit -m "roder ${version}"
git push origin HEAD
echo "update-homebrew-tap: pushed roder ${version} to ${tap_repo} (${formula_path}, sha256 ${sha256})."
