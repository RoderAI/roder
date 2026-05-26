#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/../.."
export PATH="$HOME/.local/bin:$PATH"
export PYTHONPATH="$PWD/evals/harbor${PYTHONPATH:+:$PYTHONPATH}"

dry_run="${RODER_HARBOR_DRY_RUN:-0}"

if [[ "$dry_run" != "1" && "${RODER_HARBOR_LIVE_TBENCH:-}" != "1" ]]; then
  echo "Set RODER_HARBOR_LIVE_TBENCH=1 to run the live Harbor Terminal-Bench full suite." >&2
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
job_dir="evals/harbor/jobs/roder-tbench-full-gpt55-medium"
harbor_config="evals/harbor/tbench-full-gpt55-medium.json"
launch_plan="${RODER_HARBOR_LAUNCH_PLAN:-evals/reports/harbor/roder-tbench-full-gpt55-medium-launch-plan.json}"
pre_eval_max_age_seconds="${RODER_HARBOR_PRE_EVAL_MAX_AGE_SECONDS:-7200}"

pre_eval_summary="${RODER_HARBOR_PRE_EVAL_SUMMARY:-}"
pre_eval_campaign_summary="${RODER_HARBOR_PRE_EVAL_CAMPAIGN_SUMMARY:-}"
pre_eval_output_dir="${RODER_HARBOR_PRE_EVAL_OUTPUT_DIR:-evals/reports/pre-eval-diagnostics/full-run-latest}"
pre_eval_ran_here=0
require_pre_eval_image=1
if [[ "${RODER_HARBOR_SKIP_PREFLIGHT:-}" == "1" ]]; then
  require_pre_eval_image=0
fi

if [[ -z "$pre_eval_summary" ]]; then
  pre_eval_args=(
    --require-prebuilt
    --require-auth
    --output-dir "$pre_eval_output_dir"
  )
  pre_eval_args+=(--config "$harbor_config")
  if [[ "$require_pre_eval_image" == "1" ]]; then
    pre_eval_args+=(--preflight-images)
    if [[ "${RODER_HARBOR_PREFLIGHT_PULL:-}" == "1" ]]; then
      pre_eval_args+=(--pull-images)
    fi
  fi
  if [[ -n "${RODER_HARBOR_PRE_EVAL_ANALYSIS:-}" ]]; then
    pre_eval_args+=(--analysis "$RODER_HARBOR_PRE_EVAL_ANALYSIS")
    if [[ -n "${RODER_HARBOR_PRE_EVAL_ANALYSIS_BASELINE:-}" ]]; then
      pre_eval_args+=(--analysis-baseline "$RODER_HARBOR_PRE_EVAL_ANALYSIS_BASELINE")
    fi
  fi
  if [[ -n "$pre_eval_campaign_summary" ]]; then
    pre_eval_args+=(--campaign-summary "$pre_eval_campaign_summary")
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
  --max-age-seconds "$pre_eval_max_age_seconds"
)
summary_validation_args+=(--require-config "$harbor_config")
if [[ "$require_pre_eval_image" == "1" ]]; then
  summary_validation_args+=(--require-image-preflight)
  summary_validation_args+=(--verify-image-manifest)
  summary_validation_args+=(--require-image-config "$harbor_config")
fi
if [[ -n "${RODER_HARBOR_PRE_EVAL_ANALYSIS:-}" || "${RODER_HARBOR_PRE_EVAL_REQUIRE_ANALYSIS:-}" == "1" ]]; then
  summary_validation_args+=(--require-analysis)
fi
if [[ -n "$pre_eval_campaign_summary" ]]; then
  summary_validation_args+=(--require-campaign-summary)
  summary_validation_args+=(--campaign-summary "$pre_eval_campaign_summary")
fi
python3 evals/harbor/validate_pre_eval_summary.py "${summary_validation_args[@]}"

launch_plan_args=(
  --output "$launch_plan"
  --pre-eval-summary "$pre_eval_summary"
  --pre-eval-output-dir "$pre_eval_output_dir"
  --job-dir "$job_dir"
  --harbor-config "$harbor_config"
  --analysis-json evals/reports/harbor/roder-tbench-full-gpt55-medium-analysis.json
  --analysis-markdown evals/reports/harbor/roder-tbench-full-gpt55-medium.md
  --max-pre-eval-age-seconds "$pre_eval_max_age_seconds"
)
if [[ "$pre_eval_ran_here" == "1" ]]; then
  launch_plan_args+=(--pre-eval-ran-here)
fi
if [[ "$require_pre_eval_image" == "1" ]]; then
  launch_plan_args+=(--require-image-preflight)
fi
if [[ -n "${RODER_HARBOR_PRE_EVAL_ANALYSIS:-}" ]]; then
  launch_plan_args+=(--analysis-target "$RODER_HARBOR_PRE_EVAL_ANALYSIS")
fi
if [[ -n "${RODER_HARBOR_PRE_EVAL_ANALYSIS:-}" || "${RODER_HARBOR_PRE_EVAL_REQUIRE_ANALYSIS:-}" == "1" ]]; then
  launch_plan_args+=(--require-analysis)
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
if [[ -n "$pre_eval_campaign_summary" ]]; then
  launch_plan_args+=(--campaign-summary "$pre_eval_campaign_summary")
fi
python3 evals/harbor/write_tbench_launch_plan.py "${launch_plan_args[@]}"
printf 'Full run launch plan written: %s\n' "$launch_plan"

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
  if [[ -n "$pre_eval_campaign_summary" ]]; then
    dry_run_validator_args+=(--require-campaign-summary)
  fi
  python3 evals/harbor/validate_tbench_launch_plan.py "${dry_run_validator_args[@]}"
  printf 'Full run dry-run passed: pre-eval summary=%s\n' "$pre_eval_summary"
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
if [[ -n "$pre_eval_campaign_summary" ]]; then
  launch_validator_args+=(--require-campaign-summary)
fi
if ! python3 evals/harbor/validate_tbench_launch_plan.py "${launch_validator_args[@]}"; then
  exit 2
fi

if [[ "${RODER_HARBOR_SKIP_PREFLIGHT:-}" != "1" ]]; then
  pre_eval_manifest="$pre_eval_output_dir/image-preflight/manifest.json"
  if [[ "$pre_eval_ran_here" == "1" && -f "$pre_eval_manifest" ]]; then
    cp "$pre_eval_manifest" evals/reports/harbor/roder-tbench-full-gpt55-medium-images.json
  elif [[ "${RODER_HARBOR_PREFLIGHT_PULL:-}" == "1" ]]; then
    python3 evals/harbor/preflight_tbench_images.py \
      --config "$harbor_config" \
      --pull \
      --manifest evals/reports/harbor/roder-tbench-full-gpt55-medium-images.json
  else
    python3 evals/harbor/preflight_tbench_images.py \
      --config "$harbor_config" \
      --offline \
      --manifest evals/reports/harbor/roder-tbench-full-gpt55-medium-images.json
  fi
fi

if [[ -e "$job_dir" ]]; then
  if [[ "${RODER_HARBOR_REPLACE_JOB:-}" != "1" ]]; then
    echo "$job_dir already exists. Set RODER_HARBOR_REPLACE_JOB=1 to replace it." >&2
    exit 2
  fi
  rm -rf "$job_dir"
fi

harbor run --config "$harbor_config"

python3 evals/harbor/analyze_tbench_run.py \
  "$job_dir" \
  --require-clean \
  --json evals/reports/harbor/roder-tbench-full-gpt55-medium-analysis.json \
  --markdown evals/reports/harbor/roder-tbench-full-gpt55-medium.md \
  --manifest-dir evals/reports/harbor/manifests \
  --group-scored-failures

python3 evals/harbor/validate_tbench_analysis.py \
  evals/reports/harbor/roder-tbench-full-gpt55-medium-analysis.json \
  --baseline evals/harbor/tbench-clean-baseline.json \
  --markdown evals/reports/harbor/roder-tbench-full-gpt55-medium-baseline.md
