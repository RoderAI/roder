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
python3 evals/harbor/validate_pre_eval_summary.py "${summary_validation_args[@]}"

write_launch_plan() {
  local output_path="$1"
  python3 - "$output_path" \
    "$pre_eval_summary" \
    "$pre_eval_output_dir" \
    "$pre_eval_ran_here" \
    "$require_pre_eval_image" \
    "${RODER_HARBOR_PRE_EVAL_ANALYSIS:-}" \
    "${RODER_HARBOR_SKIP_PREFLIGHT:-0}" \
    "${RODER_HARBOR_PREFLIGHT_PULL:-0}" \
    "${RODER_HARBOR_REPLACE_JOB:-0}" \
    "$dry_run" \
    "$job_dir" \
    "$harbor_config" \
    "$pre_eval_max_age_seconds" <<'PY'
import json
import hashlib
import sys
from datetime import datetime, timezone
from pathlib import Path

(
    output_path,
    pre_eval_summary,
    pre_eval_output_dir,
    pre_eval_ran_here,
    require_pre_eval_image,
    analysis_target,
    skip_preflight,
    pull_preflight,
    replace_job,
    dry_run,
    job_dir,
    harbor_config,
    max_pre_eval_age_seconds,
) = sys.argv[1:]

is_dry_run = dry_run == "1"
job_path = Path(job_dir)
job_dir_exists = job_path.exists()
job_dir_blocks_launch = (not is_dry_run) and job_dir_exists and replace_job != "1"
blocked_reasons = ["existing_job_dir"] if job_dir_blocks_launch else []
blocked_before_harbor = blocked_reasons[0] if blocked_reasons else None
launch_status = (
    "dry_run"
    if is_dry_run
    else "blocked"
    if blocked_reasons
    else "ready"
)
try:
    summary_bytes = Path(pre_eval_summary).read_bytes()
    summary = json.loads(summary_bytes)
    summary_sha256 = hashlib.sha256(summary_bytes).hexdigest()
    if not isinstance(summary, dict):
        summary = {}
except Exception:
    summary = {}
    summary_sha256 = None
effective_pre_eval_output_dir = pre_eval_output_dir
if isinstance(summary.get("outputDir"), str) and summary.get("outputDir"):
    effective_pre_eval_output_dir = summary["outputDir"]
summary_options = summary.get("options") if isinstance(summary.get("options"), dict) else {}
effective_require_pre_eval_image = require_pre_eval_image == "1"
if isinstance(summary_options.get("preflightImages"), bool):
    effective_require_pre_eval_image = summary_options["preflightImages"]
effective_pull_preflight = pull_preflight == "1"
if isinstance(summary_options.get("pullImages"), bool):
    effective_pull_preflight = summary_options["pullImages"]
try:
    harbor_config_sha256 = hashlib.sha256(Path(harbor_config).read_bytes()).hexdigest()
except OSError:
    harbor_config_sha256 = None
git = summary.get("git") if isinstance(summary.get("git"), dict) else {}
harbor_configs = (
    summary.get("checks", {}).get("harborConfigs", {})
    if isinstance(summary.get("checks"), dict)
    else {}
)
pre_eval_harbor_config_sha256 = None
if isinstance(harbor_configs, dict) and isinstance(harbor_configs.get("entries"), list):
    for entry in harbor_configs["entries"]:
        if not isinstance(entry, dict):
            continue
        if entry.get("path") == harbor_config and isinstance(entry.get("sha256"), str):
            pre_eval_harbor_config_sha256 = entry.get("sha256")
            break
prebuilt = summary.get("prebuiltBinary") if isinstance(summary.get("prebuiltBinary"), dict) else {}
prebuilt_binary = {
    key: prebuilt[key]
    for key in (
        "path",
        "sha256",
        "sizeBytes",
        "modifiedAt",
        "fileType",
        "linuxX8664Elf",
        "executable",
    )
    if key in prebuilt
}
auth = summary.get("authFile") if isinstance(summary.get("authFile"), dict) else {}
auth_file = {
    key: auth[key]
    for key in (
        "path",
        "sizeBytes",
        "modifiedAt",
        "validJson",
        "jsonFields",
    )
    if key in auth
}
image_preflight = (
    summary.get("checks", {}).get("imagePreflight")
    if isinstance(summary.get("checks"), dict)
    and isinstance(summary.get("checks", {}).get("imagePreflight"), dict)
    else {}
)
harbor_harness = (
    summary.get("checks", {}).get("harborHarness")
    if isinstance(summary.get("checks"), dict)
    and isinstance(summary.get("checks", {}).get("harborHarness"), dict)
    else {}
)
harbor_harness_tests = (
    summary.get("checks", {}).get("harborHarnessTests")
    if isinstance(summary.get("checks"), dict)
    and isinstance(summary.get("checks", {}).get("harborHarnessTests"), dict)
    else {}
)
summary_status = {
    "status": summary.get("status"),
    "blockedChecks": summary.get("blockedChecks")
    if isinstance(summary.get("blockedChecks"), list)
    else [],
}
if summary.get("generatedAt"):
    summary_status["generatedAt"] = summary.get("generatedAt")
if git.get("head"):
    summary_status["gitHead"] = git.get("head")
plan = {
    "generatedAt": datetime.now(timezone.utc).isoformat(),
    "launchStatus": launch_status,
    "blockedReasons": blocked_reasons,
    "dryRun": is_dry_run,
    "wouldRunHarbor": (not is_dry_run) and not job_dir_blocks_launch,
    "harborConfig": harbor_config,
    "harborConfigSha256": harbor_config_sha256,
    "preEvalHarborConfigSha256": pre_eval_harbor_config_sha256,
    "jobDir": job_dir,
    "jobDirExists": job_dir_exists,
    "jobDirBlocksLaunch": job_dir_blocks_launch,
    "blockedBeforeHarbor": blocked_before_harbor,
    "analysisJson": "evals/reports/harbor/roder-tbench-full-gpt55-medium-analysis.json",
    "analysisMarkdown": "evals/reports/harbor/roder-tbench-full-gpt55-medium.md",
    "preEvalSummary": pre_eval_summary,
    "preEvalSummarySha256": summary_sha256,
    "preEvalSummaryStatus": summary_status,
    "prebuiltBinary": prebuilt_binary,
    "authFile": auth_file,
    "imagePreflight": image_preflight,
    "harborHarness": harbor_harness,
    "harborHarnessTests": harbor_harness_tests,
    "preEvalOutputDir": effective_pre_eval_output_dir,
    "preEvalRanHere": pre_eval_ran_here == "1",
    "requireImagePreflight": effective_require_pre_eval_image,
    "requireAnalysis": bool(analysis_target),
    "maxPreEvalAgeSeconds": int(max_pre_eval_age_seconds),
    "skipPreflight": skip_preflight == "1",
    "pullPreflight": effective_pull_preflight,
    "replaceJob": replace_job == "1",
}
path = Path(output_path)
path.parent.mkdir(parents=True, exist_ok=True)
path.write_text(json.dumps(plan, indent=2) + "\n")
PY
}

write_launch_plan "$launch_plan"
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
