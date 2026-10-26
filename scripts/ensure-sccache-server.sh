#!/bin/sh
# Start this repo's sccache server, and replace it if a different version is
# already bound to SCCACHE_SERVER_PORT. A mismatched server prints
# "error reading compile response from server" and compiles locally, which can
# leave empty object files when the disk or /tmp is tight.
set -eu

port="${SCCACHE_SERVER_PORT:-4228}"
export SCCACHE_SERVER_PORT="$port"
export SCCACHE_IDLE_TIMEOUT="${SCCACHE_IDLE_TIMEOUT:-0}"

if ! command -v sccache >/dev/null 2>&1; then
    exit 0
fi

if sccache --start-server >/dev/null 2>&1; then
    exit 0
fi

client=$(sccache --version 2>/dev/null | awk 'NR==1 { print $2 }')
server=$(sccache --show-stats 2>/dev/null | awk '/Version \(client\)/ { print $3 }')
if [ -n "$server" ] && [ "$server" != "$client" ]; then
    sccache --stop-server >/dev/null 2>&1 || true
    sccache --start-server >/dev/null 2>&1 || true
fi
