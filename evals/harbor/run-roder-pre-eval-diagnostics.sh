#!/usr/bin/env bash
set -Eeuo pipefail

cd "$(dirname "$0")/../.."
export PATH="$HOME/.local/bin:$PATH"

include_speed=0
run_tests=1
require_prebuilt=0
require_auth=0
preflight_images=0
pull_images=0
offline_images=0
output_root=""
analysis_target=""
analysis_baseline="evals/harbor/tbench-clean-baseline.json"
analysis_baseline_set=0
analysis_dir=""
speed_dir=""
auth_file="${RODER_HARBOR_AUTH_FILE:-$HOME/.roder/auth/codex.json}"
auth_file_set=0
image_preflight_config="evals/harbor/tbench-full-gpt55-medium.json"
image_preflight_config_set=0
image_preflight_manifest=""
campaign_summary=""
extra_config_paths=()
config_attestation_paths=()
harbor_readiness_status="passed"
harbor_harness_tests_status="not_run"
roder_evals_status="not_run"
summary_written=0
current_step=""

usage() {
  cat <<'EOF'
usage: evals/harbor/run-roder-pre-eval-diagnostics.sh [--include-speed] [--require-prebuilt] [--require-auth] [--auth-file PATH] [--preflight-images] [--offline-images] [--pull-images] [--image-config PATH] [--config PATH] [--analysis PATH] [--analysis-baseline PATH] [--campaign-summary PATH] [--skip-tests] [--output-dir DIR]

Runs the small local diagnostic loop before spending a Harbor Terminal-Bench run.

Default checks:
  - Harbor config readiness validation
  - python3 -m unittest discover -s evals/harbor -p 'test_*.py'
  - cargo test -p roder-evals --lib
  - roder eval run evals/fixtures/tbench-diagnostics --offline --profile eval

Optional:
  --include-speed  also run evals/fixtures/speed with --speed-policy both
  --require-prebuilt
                   require evals/harbor/artifacts/roder-linux-amd64 or RODER_HARBOR_PREBUILT_BINARY
  --require-auth   require the Codex auth file Harbor will upload into the task container
  --auth-file PATH auth file for --require-auth (default: RODER_HARBOR_AUTH_FILE or ~/.roder/auth/codex.json)
  --preflight-images
                   run Terminal-Bench Docker image preflight and include the manifest
  --offline-images
                   with --preflight-images, require explicit task_names and avoid registry metadata fetches
  --pull-images    with --preflight-images, pull missing images; requires explicit opt-in
  --image-config PATH
                   Harbor config for image preflight (default: evals/harbor/tbench-full-gpt55-medium.json)
  --config PATH    additional Harbor config to validate and record in the handoff summary
  --analysis PATH  validate a prior Harbor analyzer JSON or job dir against the clean-run baseline
  --analysis-baseline PATH
                   baseline JSON for --analysis (default: evals/harbor/tbench-clean-baseline.json)
  --campaign-summary PATH
                   validate and record a combined campaign summary handoff
  --skip-tests     skip Harbor Python and roder-evals unit test gates
  --output-dir DIR write reports under DIR instead of evals/reports/pre-eval-diagnostics/<timestamp>
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --include-speed)
      include_speed=1
      shift
      ;;
    --skip-tests)
      run_tests=0
      shift
      ;;
    --require-prebuilt)
      require_prebuilt=1
      shift
      ;;
    --require-auth)
      require_auth=1
      shift
      ;;
    --auth-file)
      if [[ $# -lt 2 ]]; then
        echo "--auth-file requires a value" >&2
        exit 2
      fi
      auth_file="$2"
      auth_file_set=1
      shift 2
      ;;
    --preflight-images)
      preflight_images=1
      shift
      ;;
    --pull-images)
      pull_images=1
      shift
      ;;
    --offline-images)
      offline_images=1
      shift
      ;;
    --image-config)
      if [[ $# -lt 2 ]]; then
        echo "--image-config requires a value" >&2
        exit 2
      fi
      image_preflight_config="$2"
      image_preflight_config_set=1
      shift 2
      ;;
    --config)
      if [[ $# -lt 2 || "$2" == --* ]]; then
        echo "--config requires a value" >&2
        exit 2
      fi
      extra_config_paths+=("$2")
      shift 2
      ;;
    --analysis)
      if [[ $# -lt 2 ]]; then
        echo "--analysis requires a value" >&2
        exit 2
      fi
      analysis_target="$2"
      shift 2
      ;;
    --analysis-baseline)
      if [[ $# -lt 2 ]]; then
        echo "--analysis-baseline requires a value" >&2
        exit 2
      fi
      analysis_baseline="$2"
      analysis_baseline_set=1
      shift 2
      ;;
    --campaign-summary)
      if [[ $# -lt 2 ]]; then
        echo "--campaign-summary requires a value" >&2
        exit 2
      fi
      campaign_summary="$2"
      shift 2
      ;;
    --output-dir)
      if [[ $# -lt 2 ]]; then
        echo "--output-dir requires a value" >&2
        exit 2
      fi
      output_root="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ "$pull_images" == "1" && "$preflight_images" != "1" ]]; then
  echo "--pull-images requires --preflight-images" >&2
  exit 2
fi

if [[ "$offline_images" == "1" && "$preflight_images" != "1" ]]; then
  echo "--offline-images requires --preflight-images" >&2
  exit 2
fi

if [[ "$offline_images" == "1" && "$pull_images" == "1" ]]; then
  echo "--offline-images cannot be combined with --pull-images" >&2
  exit 2
fi

if [[ "$analysis_baseline_set" == "1" && -z "$analysis_target" ]]; then
  echo "--analysis-baseline requires --analysis" >&2
  exit 2
fi

if [[ "$image_preflight_config_set" == "1" && "$preflight_images" != "1" ]]; then
  echo "--image-config requires --preflight-images" >&2
  exit 2
fi

if [[ "$auth_file_set" == "1" && "$require_auth" != "1" ]]; then
  echo "--auth-file requires --require-auth" >&2
  exit 2
fi

append_config_attestation_path() {
  local candidate="$1"
  local existing
  if [[ ${#config_attestation_paths[@]} -gt 0 ]]; then
    for existing in "${config_attestation_paths[@]}"; do
      if [[ "$existing" == "$candidate" ]]; then
        return 0
      fi
    done
  fi
  config_attestation_paths+=("$candidate")
}

if [[ "$preflight_images" == "1" || ${#extra_config_paths[@]} -gt 0 ]]; then
  append_config_attestation_path "evals/harbor/tbench-full-gpt55-medium.json"
  append_config_attestation_path "evals/harbor/tbench-smoke.json"
  if [[ "$preflight_images" == "1" && -n "$image_preflight_config" ]]; then
    append_config_attestation_path "$image_preflight_config"
  fi
  if [[ ${#extra_config_paths[@]} -gt 0 ]]; then
    for config_path in "${extra_config_paths[@]}"; do
      append_config_attestation_path "$config_path"
    done
  fi
fi

if [[ -z "$output_root" ]]; then
  stamp="$(date -u '+%Y%m%dT%H%M%SZ')"
  output_root="evals/reports/pre-eval-diagnostics/$stamp"
fi

mkdir -p "$output_root"
if [[ "$run_tests" == "0" ]]; then
  harbor_harness_tests_status="skipped"
  roder_evals_status="skipped"
fi

run_step() {
  current_step="$*"
  printf '\n==> %s\n' "$*"
  "$@"
}

run_step_without_harbor_run_control_env() {
  current_step="$*"
  printf '\n==> %s\n' "$*"
  env \
    -u RODER_HARBOR_DRY_RUN \
    -u RODER_HARBOR_LIVE_TBENCH \
    -u RODER_HARBOR_REPLACE_JOB \
    -u RODER_HARBOR_SKIP_PREFLIGHT \
    -u RODER_HARBOR_PRE_EVAL_SUMMARY \
    -u RODER_HARBOR_PRE_EVAL_CAMPAIGN_SUMMARY \
    -u RODER_HARBOR_PRE_EVAL_ANALYSIS \
    -u RODER_HARBOR_PRE_EVAL_REQUIRE_ANALYSIS \
    "$@"
}

write_summary() {
  if [[ "$summary_written" == "1" ]]; then
    return 0
  fi
  summary_written=1
  summary_path="$output_root/pre-eval-summary.json"
  summary_args=(
    --summary "$summary_path"
    --output-dir "$output_root"
    --tbench-dir "$output_root/tbench-diagnostics"
    --analysis-baseline "$analysis_baseline"
    --prebuilt-binary "${RODER_HARBOR_PREBUILT_BINARY:-evals/harbor/artifacts/roder-linux-amd64}"
    --auth-file "$auth_file"
    --harbor-readiness-status "$harbor_readiness_status"
    --harbor-harness-tests-status "$harbor_harness_tests_status"
    --roder-evals-status "$roder_evals_status"
  )
  if [[ "$run_tests" == "1" ]]; then
    summary_args+=(--run-tests)
  fi
  if [[ "$include_speed" == "1" ]]; then
    summary_args+=(--include-speed --speed-dir "$speed_dir")
  fi
  if [[ "$require_prebuilt" == "1" ]]; then
    summary_args+=(--require-prebuilt)
  fi
  if [[ "$require_auth" == "1" ]]; then
    summary_args+=(--require-auth)
  fi
  if [[ "$preflight_images" == "1" ]]; then
    summary_args+=(--preflight-images)
  fi
  if [[ "$offline_images" == "1" ]]; then
    summary_args+=(--offline-images)
  fi
  if [[ "$pull_images" == "1" ]]; then
    summary_args+=(--pull-images)
  fi
  if [[ -n "$image_preflight_config" ]]; then
    summary_args+=(--image-config "$image_preflight_config")
  fi
  if [[ ${#config_attestation_paths[@]} -gt 0 ]]; then
    for config_path in "${config_attestation_paths[@]}"; do
      summary_args+=(--config "$config_path")
    done
  fi
  if [[ -n "$analysis_target" && -n "$analysis_dir" ]]; then
    summary_args+=(--analysis-target "$analysis_target" --analysis-dir "$analysis_dir")
  fi
  if [[ -n "$image_preflight_manifest" ]]; then
    summary_args+=(--image-manifest "$image_preflight_manifest")
  fi
  if [[ -n "$campaign_summary" ]]; then
    summary_args+=(--campaign-summary "$campaign_summary")
  fi
  run_step python3 evals/harbor/write_pre_eval_summary.py "${summary_args[@]}" "$@"
}

on_error() {
  exit_code="$1"
  trap - ERR
  write_summary --failure-step "$current_step" --failure-exit-code "$exit_code" || true
  exit "$exit_code"
}

trap 'on_error $?' ERR

readiness_args=()
if [[ "$require_prebuilt" == "1" ]]; then
  readiness_args+=(--require-prebuilt)
fi
if [[ "$require_auth" == "1" ]]; then
  readiness_args+=(--require-auth --auth-file "$auth_file")
fi
if [[ ${#config_attestation_paths[@]} -gt 0 ]]; then
  for config_path in "${config_attestation_paths[@]}"; do
    readiness_args+=(--config "$config_path")
  done
fi
if [[ ${#readiness_args[@]} -gt 0 ]]; then
  harbor_readiness_status="failed"
  run_step python3 evals/harbor/validate_harbor_readiness.py "${readiness_args[@]}"
else
  harbor_readiness_status="failed"
  run_step python3 evals/harbor/validate_harbor_readiness.py
fi
harbor_readiness_status="passed"

if [[ "$run_tests" == "1" ]]; then
  harbor_harness_tests_status="failed"
  run_step_without_harbor_run_control_env python3 -m unittest discover -s evals/harbor -p 'test_*.py'
  harbor_harness_tests_status="passed"
  roder_evals_status="failed"
  run_step cargo test -p roder-evals --lib
  roder_evals_status="passed"
else
  roder_evals_status="skipped"
fi

tbench_dir="$output_root/tbench-diagnostics"
run_step env RODER_EVAL_OUTPUT_DIR="$tbench_dir" \
  cargo run -p roder-cli -- eval run evals/fixtures/tbench-diagnostics --offline --profile eval

run_step python3 evals/harbor/validate_pre_eval_tbench_diagnostics.py \
  "$tbench_dir/eval-run.json"

if [[ -n "$analysis_target" ]]; then
  analysis_dir="$output_root/harbor-analysis-baseline"
  mkdir -p "$analysis_dir"
  run_step python3 evals/harbor/validate_tbench_analysis.py \
    "$analysis_target" \
    --baseline "$analysis_baseline" \
    --json "$analysis_dir/validation.json" \
    --markdown "$analysis_dir/validation.md"
fi

if [[ "$preflight_images" == "1" ]]; then
  image_preflight_dir="$output_root/image-preflight"
  mkdir -p "$image_preflight_dir"
  image_preflight_manifest="$image_preflight_dir/manifest.json"
  if [[ "$pull_images" == "1" ]]; then
    run_step env RODER_HARBOR_PREFLIGHT_PULL=1 \
      python3 evals/harbor/preflight_tbench_images.py \
      --config "$image_preflight_config" \
      --pull \
      --manifest "$image_preflight_manifest"
  elif [[ "$offline_images" == "1" ]]; then
    run_step python3 evals/harbor/preflight_tbench_images.py \
      --config "$image_preflight_config" \
      --offline \
      --manifest "$image_preflight_manifest"
  else
    run_step python3 evals/harbor/preflight_tbench_images.py \
      --config "$image_preflight_config" \
      --manifest "$image_preflight_manifest"
  fi
fi

if [[ "$include_speed" == "1" ]]; then
  speed_dir="$output_root/speed-policy"
  run_step env RODER_EVAL_OUTPUT_DIR="$speed_dir" \
    cargo run -p roder-cli -- eval run evals/fixtures/speed --offline --speed-policy both
fi

write_summary --require-ok

printf '\nPre-eval diagnostics complete: %s\n' "$output_root"
