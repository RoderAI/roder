#!/usr/bin/env bash
set -euo pipefail

lab_dir="${1:-${RODER_ROUTING_LAB:-/tmp/roder-routing-lab}}"
follow="${2:-}"

if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required for formatted routing logs." >&2
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
  cat >&2 <<EOF
No Roder event log found under:
  $lab_dir/threads

Start a TUI turn first:
  cargo run -p roder -- --config-dir "$lab_dir"
EOF
  exit 1
fi

echo "Reading routing events from:"
echo "  $events_file"
echo

jq_filter='
  def selection($s): "\($s.provider // "?")/\($s.model // "?")";
  select(.kind == "inference.routing_decision" or .kind == "inference.started") |
  if .kind == "inference.routing_decision" then
    .event.InferenceRoutingDecision as $e |
    ($e.decision.matchedSignals // [] | map("\(.key):\(.value)") | join(",")) as $signals |
    "ROUTE turn=\($e.turn_id) round=\($e.round_index) outcome=\($e.decision.outcome) tier=\($e.decision.metadata.tier // "-") default=\(selection($e.default_selection)) selected=\(selection($e.selected_selection)) reason=\"\($e.decision.reason)\" savings=\($e.decision.costDelta.estimatedSavingsUsd // "n/a") signals=\($signals)"
  else
    .event.InferenceStarted as $e |
    "START turn=\($e.turn_id) engine=\($e.engine_id) model=\(selection($e.model)) reasoning=\($e.reasoning.level // "none")"
  end
'

if [[ "$follow" == "--follow" || "$follow" == "-f" ]]; then
  tail -n "${RODER_ROUTING_TAIL_LINES:-200}" -f "$events_file" | jq --unbuffered -r "$jq_filter"
else
  tail -n "${RODER_ROUTING_TAIL_LINES:-2000}" "$events_file" | jq -r "$jq_filter"
fi
