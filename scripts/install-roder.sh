#!/usr/bin/env bash
set -euo pipefail

base_url="${RODER_DOWNLOAD_BASE_URL:-https://dl.roder.sh/latest}"
install_dir="${RODER_INSTALL_DIR:-$HOME/.local/bin}"
install_name="${RODER_INSTALL_NAME:-roder}"
target="${RODER_TARGET:-}"
version="${RODER_VERSION:-latest}"
archive_format="${RODER_ARCHIVE_FORMAT:-tar.gz}"
force_direct="${RODER_FORCE_DIRECT_INSTALL:-0}"
tmp_dir=""

usage() {
  cat <<'USAGE'
Install Roder.

Usage:
  curl -fsSL https://dl.roder.sh/install.sh | bash
  curl -fsSL https://dl.roder.sh/latest/install.sh | bash

Behavior:
  macOS: installs with Homebrew (`brew install RoderAI/tap/roder`).
  Linux: downloads a verified archive from dl.roder.sh and installs `roder`.

Environment:
  RODER_INSTALL_DIR          install directory for direct installs, default: $HOME/.local/bin
  RODER_INSTALL_NAME         installed binary name, default: roder
  RODER_DOWNLOAD_BASE_URL    download base URL, default: https://dl.roder.sh/latest
  RODER_TARGET               override target triple, e.g. x86_64-unknown-linux-gnu
  RODER_ARCHIVE_FORMAT       tar.gz or zip, default: tar.gz
  RODER_FORCE_DIRECT_INSTALL set to 1 on macOS to skip Homebrew and download directly
  RODER_HOMEBREW_TAP         Homebrew tap, default: RoderAI/tap
  RODER_HOMEBREW_FORMULA     Homebrew formula, default: RoderAI/tap/roder
USAGE
}

log() {
  printf 'roder install: %s\n' "$*" >&2
}

fail() {
  printf 'roder install: error: %s\n' "$*" >&2
  exit 1
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "$1 is required"
}

download() {
  local url="$1"
  local out="$2"
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL --retry 3 --retry-delay 1 --connect-timeout 20 "$url" -o "$out"
  elif command -v wget >/dev/null 2>&1; then
    wget -q "$url" -O "$out"
  else
    fail "curl or wget is required"
  fi
}

sha256_file() {
  local path="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$path" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$path" | awk '{print $1}'
  else
    fail "sha256sum or shasum is required"
  fi
}

normalize_target() {
  local os arch os_part arch_part
  os="$(uname -s)"
  arch="$(uname -m)"
  case "$os" in
    Linux) os_part="unknown-linux-gnu" ;;
    Darwin) os_part="apple-darwin" ;;
    *) fail "unsupported OS: $os" ;;
  esac
  case "$arch" in
    x86_64|amd64) arch_part="x86_64" ;;
    aarch64|arm64) arch_part="aarch64" ;;
    *) fail "unsupported architecture: $arch" ;;
  esac
  printf '%s-%s\n' "$arch_part" "$os_part"
}

install_with_homebrew() {
  local tap="${RODER_HOMEBREW_TAP:-RoderAI/tap}"
  local formula="${RODER_HOMEBREW_FORMULA:-RoderAI/tap/roder}"
  local formula_name="${formula##*/}"
  if ! command -v brew >/dev/null 2>&1; then
    fail "Homebrew is required on macOS. Install it from https://brew.sh, then run: brew install ${formula}. To force a direct archive install, set RODER_FORCE_DIRECT_INSTALL=1."
  fi

  if [[ "$formula" != */* ]] && ! brew tap | grep -qx "$tap"; then
    log "tapping Homebrew repository $tap"
    brew tap "$tap"
  fi

  if brew list --formula "$formula_name" >/dev/null 2>&1; then
    log "upgrading Homebrew formula $formula"
    brew upgrade "$formula" || brew reinstall "$formula"
  else
    log "installing Homebrew formula $formula"
    brew install "$formula"
  fi

  log "installed via Homebrew: $(command -v roder || true)"
}

install_direct_archive() {
  if [[ -z "$target" ]]; then
    target="$(normalize_target)"
  fi

  case "$archive_format" in
    tar.gz|tgz) archive_format="tar.gz" ;;
    zip) ;;
    *) fail "RODER_ARCHIVE_FORMAT must be tar.gz or zip" ;;
  esac

  local archive="roder-${target}.${archive_format}"
  local archive_url="${base_url%/}/${archive}"
  local checksum_url="${archive_url}.sha256"
  local checksums_url="${base_url%/}/SHA256SUMS"
  local expected actual extracted install_tmp
  tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/roder-install.XXXXXX")"
  cleanup() { rm -rf "${tmp_dir:-}"; }
  trap cleanup EXIT INT TERM

  log "downloading ${archive_url}"
  download "$archive_url" "$tmp_dir/$archive"
  if ! download "$checksum_url" "$tmp_dir/$archive.sha256"; then
    log "per-archive checksum not found; falling back to SHA256SUMS"
    download "$checksums_url" "$tmp_dir/SHA256SUMS"
    awk -v name="$archive" '$2 == name { print; found = 1 } END { exit found ? 0 : 1 }' "$tmp_dir/SHA256SUMS" > "$tmp_dir/$archive.sha256" || fail "checksum not found for $archive"
  fi

  expected="$(awk '{print $1}' "$tmp_dir/$archive.sha256")"
  actual="$(sha256_file "$tmp_dir/$archive")"
  if [[ "$actual" != "$expected" ]]; then
    fail "checksum mismatch for $archive (expected $expected, got $actual)"
  fi

  mkdir -p "$tmp_dir/extract"
  case "$archive_format" in
    tar.gz)
      require_cmd tar
      tar -xzf "$tmp_dir/$archive" -C "$tmp_dir/extract"
      ;;
    zip)
      require_cmd unzip
      unzip -q "$tmp_dir/$archive" -d "$tmp_dir/extract"
      ;;
  esac

  extracted="$(find "$tmp_dir/extract" -type f -name roder -perm -u+x | head -n 1)"
  if [[ -z "$extracted" ]]; then
    extracted="$(find "$tmp_dir/extract" -type f -name roder | head -n 1)"
  fi
  [[ -n "$extracted" ]] || fail "archive did not contain a roder binary"

  mkdir -p "$install_dir"
  install_tmp="$install_dir/.${install_name}.tmp.$$"
  install -m 0755 "$extracted" "$install_tmp"
  mv "$install_tmp" "$install_dir/$install_name"

  log "installed ${install_dir}/${install_name} (${version}, ${target})"
  case ":$PATH:" in
    *":$install_dir:"*) ;;
    *) log "add ${install_dir} to PATH to run ${install_name} directly" ;;
  esac
}

case "$(uname -s)" in
  Darwin)
    if [[ "$force_direct" == "1" ]]; then
      install_direct_archive
    else
      install_with_homebrew
    fi
    ;;
  Linux)
    install_direct_archive
    ;;
  *)
    fail "unsupported OS: $(uname -s)"
    ;;
esac
