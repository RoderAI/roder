#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/../.."
export PATH="$HOME/.local/bin:$PATH"
export PYTHONPATH="$PWD/evals/harbor${PYTHONPATH:+:$PYTHONPATH}"

if [[ "${RODER_HARBOR_LIVE_TBENCH:-}" != "1" ]]; then
  echo "Set RODER_HARBOR_LIVE_TBENCH=1 to run the live Harbor Terminal-Bench full suite." >&2
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
      --config evals/harbor/tbench-full-gpt55-medium.json \
      --pull \
      --manifest evals/reports/harbor/roder-tbench-full-gpt55-medium-images.json
  else
    python3 evals/harbor/preflight_tbench_images.py \
      --config evals/harbor/tbench-full-gpt55-medium.json \
      --offline \
      --manifest evals/reports/harbor/roder-tbench-full-gpt55-medium-images.json
  fi
fi

job_dir="evals/harbor/jobs/roder-tbench-full-gpt55-medium"
if [[ -e "$job_dir" ]]; then
  if [[ "${RODER_HARBOR_REPLACE_JOB:-}" != "1" ]]; then
    echo "$job_dir already exists. Set RODER_HARBOR_REPLACE_JOB=1 to replace it." >&2
    exit 2
  fi
  rm -rf "$job_dir"
fi

harbor run --config evals/harbor/tbench-full-gpt55-medium.json

python3 evals/harbor/analyze_tbench_run.py \
  "$job_dir" \
  --require-clean \
  --json evals/reports/harbor/roder-tbench-full-gpt55-medium-analysis.json \
  --markdown evals/reports/harbor/roder-tbench-full-gpt55-medium.md \
  --manifest-dir evals/reports/harbor/manifests \
  --group-scored-failures
