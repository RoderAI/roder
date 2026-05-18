#!/usr/bin/env bash
set -euo pipefail

session="${RODER_TEAM_TMUX_SESSION:-roder-team-smoke}"
out_dir="${RODER_TEAM_TMUX_OUT_DIR:-.roder/tmux-captures/team-smoke}"
helper=".agents/skills/roder-tmux/scripts/roder_tmux.sh"

if [[ "${RODER_LIVE_TMUX_TEAM:-}" != "1" ]]; then
  echo "set RODER_LIVE_TMUX_TEAM=1 to run the live tmux team smoke"
  exit 0
fi

if ! command -v tmux >/dev/null 2>&1; then
  echo "tmux is not available; skipping live tmux team smoke"
  exit 0
fi

mkdir -p "$out_dir"
cargo build -p roder-cli --bin roder

team_json="$(mktemp)"
printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"team/start","params":{"displayMode":"in_process","members":[{"name":"Builder"},{"name":"Reviewer"}]}}' \
  | target/debug/roder app-server > "$team_json"
team_ref="$(node -e 'const fs=require("fs"); const lines=fs.readFileSync(process.argv[1],"utf8").trim().split(/\n+/).map(JSON.parse); const res=lines.find(x=>x.id===1); if(!res || res.error) { console.error(fs.readFileSync(process.argv[1],"utf8")); process.exit(1); } const team=res.result.team; console.log(`${team.id}\t${team.members[1].id}`);' "$team_json")"
team_id="${team_ref%%	*}"
member_id="${team_ref#*	}"

"$helper" stop -s "${session}-lead" >/dev/null 2>&1 || true
"$helper" start -s "${session}-lead" -- target/debug/roder --team-display in-process
sleep 2
"$helper" capture -s "${session}-lead" -o "$out_dir"
"$helper" stop -s "${session}-lead"

"$helper" stop -s "${session}-member" >/dev/null 2>&1 || true
"$helper" start -s "${session}-member" -- target/debug/roder team attach --team "$team_id" --member "$member_id"
sleep 2
"$helper" capture -s "${session}-member" -o "$out_dir"
"$helper" stop -s "${session}-member"

echo "team smoke captures written under $out_dir"
echo "team=$team_id member=$member_id"
