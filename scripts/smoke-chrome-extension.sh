#!/usr/bin/env bash
# Offline end-to-end smoke test for the Roder Chrome browser bridge.
#
# Starts a local `roder app-server --remote` with a known token/port, then runs
# a faithful fake-extension WebSocket client (scripts/chrome-bridge-e2e.mjs) that
# pairs, advertises capabilities, and answers dispatched commands. Proves the
# whole gode path (transport push → bridge registration → chrome/* dispatch)
# without launching a real browser.
#
# Gated behind RODER_CHROME_LIVE=1 so it never runs in the normal test suite.
# Usage: RODER_CHROME_LIVE=1 ./scripts/smoke-chrome-extension.sh
set -euo pipefail

if [[ "${RODER_CHROME_LIVE:-0}" != "1" ]]; then
  echo "skipped: set RODER_CHROME_LIVE=1 to run the Chrome bridge smoke test"
  exit 0
fi

cd "$(dirname "$0")/.."

PORT="${RODER_CHROME_PORT:-17345}"
TOKEN="${RODER_CHROME_TOKEN:-chrome-e2e-$RANDOM$RANDOM}"
LISTEN="ws://127.0.0.1:${PORT}"

# Locate a roder binary, falling back to `cargo run`.
if [[ -n "${RODER_BIN:-}" ]]; then
  RUN=("$RODER_BIN")
elif [[ -x target/release/roder ]]; then
  RUN=(target/release/roder)
elif [[ -x target/debug/roder ]]; then
  RUN=(target/debug/roder)
else
  RUN=(cargo run -q -p roder-cli --)
fi

echo "starting app-server: ${RUN[*]} app-server --remote --listen $LISTEN"
"${RUN[@]}" app-server --remote --listen "$LISTEN" --auth-token "$TOKEN" >/tmp/roder-chrome-smoke.log 2>&1 &
SERVER_PID=$!
trap 'kill "$SERVER_PID" 2>/dev/null || true' EXIT

# Wait for readiness via the unauthenticated health probe.
for _ in $(seq 1 60); do
  if curl -fsS "http://127.0.0.1:${PORT}/readyz" >/dev/null 2>&1; then
    break
  fi
  sleep 0.5
done

node scripts/chrome-bridge-e2e.mjs "$LISTEN" "$TOKEN"
STATUS=$?

echo "server log tail:"
tail -n 5 /tmp/roder-chrome-smoke.log || true
exit $STATUS
