#!/usr/bin/env sh
set -eu

base_url="${RODER_DOWNLOAD_BASE_URL:-https://dl.roder.sh/latest}"
install_dir="${RODER_INSTALL_DIR:-$HOME/.local/bin}"
install_name="${RODER_INSTALL_NAME:-roder}"
target="${RODER_TARGET:-}"

usage() {
  cat <<'USAGE'
Usage:
  curl -fsSL https://dl.roder.sh/latest/install.sh | sh

Environment:
  RODER_INSTALL_DIR       install directory, default: $HOME/.local/bin
  RODER_INSTALL_NAME      installed binary name, default: roder
  RODER_DOWNLOAD_BASE_URL download base URL, default: https://dl.roder.sh/latest
  RODER_TARGET            override target triple for testing or custom installs
USAGE
}

if [ "${1:-}" = "-h" ] || [ "${1:-}" = "--help" ]; then
  usage
  exit 0
fi

if [ -z "$target" ]; then
  os="$(uname -s)"
  arch="$(uname -m)"
  case "$os" in
    Linux) os_part="unknown-linux-gnu" ;;
    Darwin) os_part="apple-darwin" ;;
    *)
      echo "roder install: unsupported OS: $os" >&2
      exit 1
      ;;
  esac
  case "$arch" in
    x86_64|amd64) arch_part="x86_64" ;;
    aarch64|arm64) arch_part="aarch64" ;;
    *)
      echo "roder install: unsupported architecture: $arch" >&2
      exit 1
      ;;
  esac
  target="${arch_part}-${os_part}"
fi

binary="roder-${target}"
tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/roder-install.XXXXXX")"
cleanup() {
  rm -rf "$tmp_dir"
}
trap cleanup EXIT INT TERM

download() {
  url="$1"
  out="$2"
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL --retry 3 --retry-delay 1 "$url" -o "$out"
  elif command -v wget >/dev/null 2>&1; then
    wget -q "$url" -O "$out"
  else
    echo "roder install: curl or wget is required" >&2
    exit 1
  fi
}

echo "roder install: downloading ${binary}"
download "${base_url}/${binary}" "${tmp_dir}/${binary}"
download "${base_url}/${binary}.sha256" "${tmp_dir}/${binary}.sha256"

(
  cd "$tmp_dir"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum -c "${binary}.sha256"
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 -c "${binary}.sha256"
  else
    echo "roder install: sha256sum or shasum is required" >&2
    exit 1
  fi
)

mkdir -p "$install_dir"
install -m 0755 "${tmp_dir}/${binary}" "${install_dir}/${install_name}"

echo "roder install: installed ${install_dir}/${install_name}"
case ":$PATH:" in
  *":$install_dir:"*) ;;
  *) echo "roder install: add ${install_dir} to PATH to run ${install_name} directly" >&2 ;;
esac
