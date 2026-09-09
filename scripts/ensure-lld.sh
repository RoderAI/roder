#!/bin/sh
# Ensure the linker .cargo/config.toml asks for is available.
#
# On Linux, cargo links through `clang -fuse-ld=lld`. Without lld on PATH every
# build fails at link time with:
#
#   clang: error: invalid linker name in argument '-fuse-ld=lld'
#
# macOS installs it from Homebrew unattended. Linux needs root, so print the
# command for this distro rather than failing the mise enter hook.

set -eu

if command -v ld.lld >/dev/null 2>&1 || command -v lld >/dev/null 2>&1; then
    exit 0
fi

case "$(uname -s)" in
Darwin)
    if command -v brew >/dev/null 2>&1; then
        brew install lld
        exit 0
    fi
    echo "roder: lld is missing and Homebrew was not found. Install Homebrew, then: brew install lld" >&2
    exit 0
    ;;
esac

if command -v pacman >/dev/null 2>&1; then
    install_cmd="sudo pacman -S --needed lld"
elif command -v apt-get >/dev/null 2>&1; then
    install_cmd="sudo apt-get install -y lld"
elif command -v dnf >/dev/null 2>&1; then
    install_cmd="sudo dnf install -y lld"
elif command -v zypper >/dev/null 2>&1; then
    install_cmd="sudo zypper install -y lld"
elif command -v apk >/dev/null 2>&1; then
    install_cmd="sudo apk add lld"
elif command -v brew >/dev/null 2>&1; then
    install_cmd="brew install lld"
else
    install_cmd="install the 'lld' package for your distribution"
fi

echo "roder: lld is missing; cargo builds will fail to link. Run: ${install_cmd}" >&2
exit 0
