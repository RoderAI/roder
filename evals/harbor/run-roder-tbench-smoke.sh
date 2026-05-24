#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/../.."
export PATH="$HOME/.local/bin:$PATH"
export PYTHONPATH="$PWD/evals/harbor${PYTHONPATH:+:$PYTHONPATH}"

if [[ "${RODER_HARBOR_LIVE_TBENCH:-}" != "1" ]]; then
  echo "Set RODER_HARBOR_LIVE_TBENCH=1 to run the live Harbor Terminal-Bench smoke." >&2
  exit 2
fi

if ! command -v harbor >/dev/null 2>&1; then
  echo "harbor is not on PATH. Install it with: uv tool install harbor" >&2
  exit 1
fi

if [[ -z "${RODER_HARBOR_PREBUILT_BINARY:-}" && ! -x evals/harbor/artifacts/roder-linux-amd64 ]]; then
  ./evals/harbor/build-prebuilt-roder.sh
fi

mkdir -p evals/reports/harbor
if [[ "${RODER_HARBOR_SKIP_PREFLIGHT:-}" != "1" ]]; then
  if [[ "${RODER_HARBOR_PREFLIGHT_PULL:-}" == "1" ]]; then
    python3 evals/harbor/preflight_tbench_images.py \
      --config evals/harbor/tbench-smoke.json \
      --pull \
      --manifest evals/reports/harbor/roder-tbench-smoke-images.json
  else
    python3 evals/harbor/preflight_tbench_images.py \
      --config evals/harbor/tbench-smoke.json \
      --offline \
      --manifest evals/reports/harbor/roder-tbench-smoke-images.json
  fi
fi

rm -rf evals/harbor/jobs/roder-tbench-smoke

harbor run --config evals/harbor/tbench-smoke.json

python3 evals/harbor/analyze_tbench_run.py \
  evals/harbor/jobs/roder-tbench-smoke \
  --require-clean \
  --markdown evals/reports/harbor/roder-tbench-smoke.md

python3 - <<'PY'
import json
from pathlib import Path

result_path = Path("evals/harbor/jobs/roder-tbench-smoke/result.json")
result = json.loads(result_path.read_text())
errors = result.get("stats", {}).get("n_errors", 0)
trials = result.get("stats", {}).get("n_trials", 0)
if errors or trials == 0:
    raise SystemExit(f"Harbor smoke failed: n_trials={trials} n_errors={errors}")
print(f"Harbor smoke passed: n_trials={trials} n_errors={errors}")
PY
