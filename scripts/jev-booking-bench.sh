#!/usr/bin/env bash
# Live booking benchmark for jev_browse through the installed `roder` binary.
#
#   scripts/jev-booking-bench.sh [--site resy|tock] [--runs N] [--model M]
#       [--reasoning R] [--prompt TEXT] [--out DIR] [--pause SECONDS]
#       [--earliest HH:MM]
#
# Each run asks Roder to find a table for 3 tonight in San Francisco's Mission
# District and to stop before signing in or confirming. Jev's session log
# goes to $OUT/sessions, the exec events to $OUT/run<i>.jsonl, and
# scripts/jev_booking_grade.py writes $OUT/summary.json.
#
# Keys live in the interactive shell, so each run goes through `zsh -ic`.
# Runs are sequential with a pause between them, to keep the request rate
# human. Jev uses its own Chrome profile (JEV_CDP_URL is unset) and the
# irreversible-action gate is on as defence in depth.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
site=resy
runs=1
model=gpt-6-sol
reasoning=low
prompt=""
out=""
pause=120
earliest=19:00

while [ $# -gt 0 ]; do
  case "$1" in
    --site) site="$2"; shift 2 ;;
    --runs) runs="$2"; shift 2 ;;
    --model) model="$2"; shift 2 ;;
    --reasoning) reasoning="$2"; shift 2 ;;
    --prompt) prompt="$2"; shift 2 ;;
    --out) out="$2"; shift 2 ;;
    --pause) pause="$2"; shift 2 ;;
    --earliest) earliest="$2"; shift 2 ;;
    -h|--help) sed -n '2,16p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

today="$(date +%F)"
entry="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))[sys.argv[2]]["entry"])' \
  "$here/jev_booking_sites.json" "$site")"
if [ -z "$prompt" ]; then
  prompt="Using jev_browse, find a table for 3 people tonight (today, $today) at a restaurant \
in San Francisco's Mission District, at 7:00 PM or later. Start at $entry. Go only as far as \
choosing one restaurant and one time slot so the reservation details show, then stop: do not \
log in, do not press Reserve or any confirm button, do not enter personal details. Reply with \
the restaurant, date, time and party size you reached."
fi

out="${out:-$PWD/jev-bench-$(date +%s)}"
mkdir -p "$out/sessions" "$out/work"
out="$(cd "$out" && pwd)"
printf '%s\n' "$prompt" > "$out/prompt.txt"

for i in $(seq 1 "$runs"); do
  [ "$i" -gt 1 ] && sleep "$pause"
  echo "run $i of $runs ($model, $reasoning) -> $out/run$i.jsonl" >&2
  (
    cd "$out/work"
    BENCH_PROMPT="$prompt" BENCH_MODEL="$model" BENCH_REASONING="$reasoning" \
    BENCH_SESSIONS="$out/sessions" \
      zsh -ic 'unset JEV_CDP_URL; JEV_SESSION_LOG="$BENCH_SESSIONS" JEV_CONFIRM_IRREVERSIBLE=1 \
        RODER_PROVIDER=codex RODER_MODEL="$BENCH_MODEL" RODER_REASONING="$BENCH_REASONING" \
        roder exec --skip-git-repo-check --ephemeral --mode accept-all --json "$BENCH_PROMPT"'
  ) > "$out/run$i.jsonl" 2> "$out/run$i.stderr" || echo "run $i exited $?" >&2
done

python3 "$here/jev_booking_grade.py" "$out" --site "$site" --today "$today" \
  --earliest "$earliest"
