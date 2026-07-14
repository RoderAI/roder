#!/bin/bash
# Run the Roder Harbor Terminal-Bench suite with the codex-parity ("minimal")
# config: benchmark guidance OFF, task ledger OFF, plan-first OFF, and NO internal
# deadline (no `timeout -s INT` wrapper, no eval_deadline_seconds) so the agent runs
# to Harbor's real per-task window exactly like the built-in Codex harness. This is
# the leaderboard-valid Codex-parity track (see roadmap/108).
#
# Usage:
#   codex login && ./evals/harbor/make-codex-access-only.sh   # once, if auth is stale
#   [INCLUDE_TASKS="terminal-bench/chess-best-move terminal-bench/path-tracing"] \
#     [N_CONCURRENT=4] ./evals/harbor/run-roder-tbench-minimal.sh
set -uo pipefail
cd "$(dirname "$0")/../.."
export PYTHONPATH="$PWD/evals/harbor${PYTHONPATH:+:$PYTHONPATH}"
export RODER_HARBOR_AUTH_FILE="${RODER_HARBOR_AUTH_FILE:-$PWD/evals/harbor/artifacts/codex-access-only.json}"

config="evals/harbor/tbench-full-gpt55-xhigh-minimal.json"

args=(run --config "$config" --jobs-dir evals/harbor/jobs --yes)
if [[ -n "${N_CONCURRENT:-}" ]]; then
  args+=(--n-concurrent "$N_CONCURRENT")
fi
for task in ${INCLUDE_TASKS:-}; do
  args+=(--include-task-name "$task")
done

harbor "${args[@]}"
echo "HARBOR_EXIT=$?"
