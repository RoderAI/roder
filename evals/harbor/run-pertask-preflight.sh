#!/bin/bash
# Targeted per-task deadline-extension preflight over clean-run regressions.
# Prereq (auth, run once when the Codex access token is stale):
#   codex login && ./evals/harbor/make-codex-access-only.sh
# Local-development evidence only (access-only auth is not leaderboard-valid).
set -uo pipefail
cd "$(dirname "$0")/../.."
export PYTHONPATH="$PWD/evals/harbor${PYTHONPATH:+:$PYTHONPATH}"
export RODER_HARBOR_AUTH_FILE="${RODER_HARBOR_AUTH_FILE:-$PWD/evals/harbor/artifacts/codex-access-only.json}"
harbor run \
  -d terminal-bench/terminal-bench-2-1 \
  -a roder_harbor_agent:RoderCli \
  -m codex/gpt-5.5 \
  --include-task-name terminal-bench/build-cython-ext \
  --include-task-name terminal-bench/chess-best-move \
  --include-task-name terminal-bench/path-tracing \
  --include-task-name terminal-bench/rstan-to-pystan \
  --include-task-name terminal-bench/path-tracing-reverse \
  --ak reasoning=xhigh \
  --ak per_task_deadlines=true \
  --ak agent_timeout_multiplier_hint=1.0 \
  --ak task_ledger_required=true \
  --ak benchmark_guidance_enabled=true \
  --ak policy_mode=bypass \
  --ak include_prebuilt_binary=true \
  --ak include_local_source=false \
  --ak policy_block_max_retries=2 \
  --job-name "${JOB_NAME:-roder-tbench-21-pertask-preflight-v2}" \
  --jobs-dir evals/harbor/jobs \
  --n-attempts 1 \
  --n-concurrent "${N_CONCURRENT:-4}" \
  --yes
echo "HARBOR_EXIT=$?"
