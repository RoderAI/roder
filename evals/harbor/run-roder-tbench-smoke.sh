#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/../.."
export PATH="$HOME/.local/bin:$PATH"
export PYTHONPATH="$PWD/evals/harbor${PYTHONPATH:+:$PYTHONPATH}"

dry_run="${RODER_HARBOR_DRY_RUN:-0}"
smoke_config="evals/harbor/tbench-smoke.json"
job_dir="evals/harbor/jobs/roder-tbench-smoke"
analysis_json="evals/reports/harbor/roder-tbench-smoke-analysis.json"
analysis_markdown="evals/reports/harbor/roder-tbench-smoke.md"
launch_plan="${RODER_HARBOR_LAUNCH_PLAN:-evals/reports/harbor/roder-tbench-smoke-launch-plan.json}"
pre_eval_max_age_seconds="${RODER_HARBOR_PRE_EVAL_MAX_AGE_SECONDS:-7200}"
pre_eval_summary="${RODER_HARBOR_PRE_EVAL_SUMMARY:-}"
pre_eval_output_dir="${RODER_HARBOR_PRE_EVAL_OUTPUT_DIR:-evals/reports/pre-eval-diagnostics/smoke-latest}"
pre_eval_ran_here=0
require_pre_eval_image=1
if [[ "${RODER_HARBOR_SKIP_PREFLIGHT:-}" == "1" ]]; then
  require_pre_eval_image=0
fi

if [[ "$dry_run" != "1" && "${RODER_HARBOR_LIVE_TBENCH:-}" != "1" ]]; then
  echo "Set RODER_HARBOR_LIVE_TBENCH=1 to run the live Harbor Terminal-Bench smoke." >&2
  exit 2
fi

if [[ "$dry_run" != "1" ]] && ! command -v harbor >/dev/null 2>&1; then
  echo "harbor is not on PATH. Install it with: uv tool install harbor" >&2
  exit 1
fi

if [[ "$dry_run" != "1" && -z "${RODER_HARBOR_PREBUILT_BINARY:-}" && ! -x evals/harbor/artifacts/roder-linux-amd64 ]]; then
  ./evals/harbor/build-prebuilt-roder.sh
fi

mkdir -p evals/reports/harbor

if [[ -z "$pre_eval_summary" ]]; then
  pre_eval_args=(
    --require-prebuilt
    --require-auth
    --output-dir "$pre_eval_output_dir"
    --config "$smoke_config"
  )
  if [[ "$require_pre_eval_image" == "1" ]]; then
    pre_eval_args+=(--preflight-images --image-config "$smoke_config")
    if [[ "${RODER_HARBOR_PREFLIGHT_PULL:-}" == "1" ]]; then
      pre_eval_args+=(--pull-images)
    else
      pre_eval_args+=(--offline-images)
    fi
  fi
  ./evals/harbor/run-roder-pre-eval-diagnostics.sh "${pre_eval_args[@]}"
  pre_eval_summary="$pre_eval_output_dir/pre-eval-summary.json"
  pre_eval_ran_here=1
fi

summary_validation_args=(
  "$pre_eval_summary"
  --require-prebuilt
  --require-auth
  --require-tests
  --verify-harbor-configs
  --verify-harness-files
  --verify-prebuilt-binary
  --verify-auth-file
  --require-config "$smoke_config"
  --max-age-seconds "$pre_eval_max_age_seconds"
)
if [[ "$require_pre_eval_image" == "1" ]]; then
  summary_validation_args+=(
    --require-image-preflight
    --verify-image-manifest
    --require-image-config "$smoke_config"
  )
fi
python3 evals/harbor/validate_pre_eval_summary.py "${summary_validation_args[@]}"

launch_plan_args=(
  --output "$launch_plan"
  --pre-eval-summary "$pre_eval_summary"
  --pre-eval-output-dir "$pre_eval_output_dir"
  --job-dir "$job_dir"
  --harbor-config "$smoke_config"
  --analysis-json "$analysis_json"
  --analysis-markdown "$analysis_markdown"
  --max-pre-eval-age-seconds "$pre_eval_max_age_seconds"
)
if [[ "$pre_eval_ran_here" == "1" ]]; then
  launch_plan_args+=(--pre-eval-ran-here)
fi
if [[ "$require_pre_eval_image" == "1" ]]; then
  launch_plan_args+=(--require-image-preflight)
fi
if [[ "${RODER_HARBOR_SKIP_PREFLIGHT:-}" == "1" ]]; then
  launch_plan_args+=(--skip-preflight)
fi
if [[ "${RODER_HARBOR_PREFLIGHT_PULL:-}" == "1" ]]; then
  launch_plan_args+=(--pull-preflight)
fi
if [[ "${RODER_HARBOR_REPLACE_JOB:-}" == "1" ]]; then
  launch_plan_args+=(--replace-job)
fi
if [[ "$dry_run" == "1" ]]; then
  launch_plan_args+=(--dry-run)
fi
python3 evals/harbor/write_tbench_launch_plan.py "${launch_plan_args[@]}"
printf 'Smoke launch plan written: %s\n' "$launch_plan"

if [[ "$dry_run" == "1" ]]; then
  dry_run_validator_args=(
    "$launch_plan"
    --allow-dry-run
    --verify-pre-eval-summary
    --verify-harbor-config
    --verify-prebuilt-binary
    --verify-auth-file
    --verify-harness-files
    --verify-image-manifest
    --max-pre-eval-age-seconds "$pre_eval_max_age_seconds"
  )
  if [[ "$require_pre_eval_image" == "1" ]]; then
    dry_run_validator_args+=(--require-image-preflight)
  fi
  python3 evals/harbor/validate_tbench_launch_plan.py "${dry_run_validator_args[@]}"
  printf 'Smoke dry-run passed: pre-eval summary=%s\n' "$pre_eval_summary"
  exit 0
fi

launch_validator_args=(
  "$launch_plan"
  --require-ready
  --verify-pre-eval-summary
  --verify-harbor-config
  --verify-prebuilt-binary
  --verify-auth-file
  --verify-harness-files
  --verify-image-manifest
  --max-pre-eval-age-seconds "$pre_eval_max_age_seconds"
)
if [[ "$require_pre_eval_image" == "1" ]]; then
  launch_validator_args+=(--require-image-preflight)
fi
if ! python3 evals/harbor/validate_tbench_launch_plan.py "${launch_validator_args[@]}"; then
  exit 2
fi

if [[ "${RODER_HARBOR_SKIP_PREFLIGHT:-}" != "1" ]]; then
  pre_eval_manifest="$pre_eval_output_dir/image-preflight/manifest.json"
  if [[ "$pre_eval_ran_here" == "1" && -f "$pre_eval_manifest" ]]; then
    cp "$pre_eval_manifest" evals/reports/harbor/roder-tbench-smoke-images.json
  elif [[ "${RODER_HARBOR_PREFLIGHT_PULL:-}" == "1" ]]; then
    python3 evals/harbor/preflight_tbench_images.py \
      --config "$smoke_config" \
      --pull \
      --manifest evals/reports/harbor/roder-tbench-smoke-images.json
  else
    python3 evals/harbor/preflight_tbench_images.py \
      --config "$smoke_config" \
      --offline \
      --manifest evals/reports/harbor/roder-tbench-smoke-images.json
  fi
fi

if [[ -e "$job_dir" ]]; then
  if [[ "${RODER_HARBOR_REPLACE_JOB:-}" != "1" ]]; then
    echo "$job_dir already exists. Set RODER_HARBOR_REPLACE_JOB=1 to replace it." >&2
    exit 2
  fi
  rm -rf "$job_dir"
fi

harbor run --config "$smoke_config"

python3 evals/harbor/analyze_tbench_run.py \
  "$job_dir" \
  --require-clean \
  --json "$analysis_json" \
  --markdown "$analysis_markdown"

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
