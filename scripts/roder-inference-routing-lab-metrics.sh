#!/usr/bin/env bash
set -euo pipefail

lab_dir="${1:-${RODER_ROUTING_LAB:-/tmp/roder-routing-lab}}"
thread_id="${2:-}"
turn_id="${3:-}"

if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required for selecting the latest turn and formatting responses." >&2
  exit 1
fi

events_file="$(
  find "$lab_dir/threads" -name events.jsonl -type f -print 2>/dev/null \
    | while IFS= read -r file; do
        mtime="$(stat -f %m "$file" 2>/dev/null || stat -c %Y "$file")"
        printf '%s\t%s\n' "$mtime" "$file"
      done \
    | sort -rn \
    | head -n 1 \
    | cut -f 2-
)"

if [[ -z "$events_file" ]]; then
  echo "No Roder event log found under $lab_dir/threads." >&2
  exit 1
fi

if [[ -z "$thread_id" ]]; then
  thread_id="$(basename "$(dirname "$events_file")")"
fi

if [[ -z "$turn_id" ]]; then
  turn_id="$(
    jq -r '
      select(.kind == "inference.routing_decision" or .kind == "turn.started") |
      .turn_id // .event.InferenceRoutingDecision.turn_id // .event.TurnStarted.turn_id // empty
    ' "$events_file" | tail -n 1
  )"
fi

if [[ -z "$turn_id" ]]; then
  echo "Could not infer a turn id from $events_file. Pass thread and turn explicitly." >&2
  exit 1
fi

printf 'Inspecting thread=%s turn=%s\n\n' "$thread_id" "$turn_id" >&2

{
  printf '{"jsonrpc":"2.0","id":1,"method":"inference/routing/status","params":{"threadId":"%s","turnId":"%s"}}\n' "$thread_id" "$turn_id"
  printf '{"jsonrpc":"2.0","id":2,"method":"inference/routing/metrics","params":{"threadId":"%s","turnId":"%s","limit":20}}\n' "$thread_id" "$turn_id"
} | cargo run -p roder -- app-server --config-dir "$lab_dir" --listen stdio:// | jq .
