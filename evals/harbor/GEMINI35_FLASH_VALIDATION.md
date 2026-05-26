# Gemini 3.5 Flash TBench Validation Subset

Use this subset for small native-Gemini validation steps before spending a full
Terminal-Bench run. It is intentionally small enough to run quickly, but still
exercises Roder's Harbor adapter, prebuilt binary injection, native Gemini API
tool calling, task ledger behavior, and verifier scoring.

## Config

- Harbor config: `evals/harbor/tbench-gemini35-flash-validation.json`
- Model: `gemini/gemini-3.5-flash`
- Reasoning: `medium`
- Parallelism: `4`
- Tasks: `6`
- Environment cleanup: disabled (`environment.delete=false`)
- Binary mode: prebuilt Linux `roder` injection

If the change under validation touches Rust code, Gemini provider behavior, CLI
exec behavior, or tool schemas, rebuild the injected Linux binary first:

```sh
./evals/harbor/build-prebuilt-roder.sh
```

Eval outputs and the prebuilt binary are gitignored under
`evals/harbor/jobs/`, `evals/reports/`, and `evals/harbor/artifacts/`.

## Task Set

| Task | Provenance | Current Result |
| --- | --- | --- |
| `configure-git-webserver` | GPT-5.5 medium pass | Pass |
| `headless-terminal` | GPT-5.5 medium pass | Pass |
| `regex-log` | GPT-5.5 medium pass | Pass |
| `sqlite-db-truncate` | GPT-5.5 medium pass | Pass |
| `kv-store-grpc` | GPT-5.5 xhigh rerun pass | Pass |
| `polyglot-c-py` | GPT-5.5 xhigh rerun pass | Fail |

`db-wal-recovery` and `query-optimize` are excluded from this small validation
subset. During Gemini harness validation, `db-wal-recovery` produced a
`RewardFileNotFoundError`, and `query-optimize` did not produce clean Harbor
scoring artifacts. They are useful debugging candidates, but they are not clean
small-subset gates.

## Current Baseline

Baseline date: May 26, 2026

- Score: `5/6`, mean `0.8333333333333334` (`83.3%`)
- Harbor errors: `0`
- Clean analysis: `true`
- Scored failures: `1` (`polyglot-c-py`)
- Run window: `2026-05-26T18:47:03.375835` to `2026-05-26T18:54:17.367113`
- Job: `evals/harbor/jobs/roder-tbench-gemini35-flash-validation`
- Analysis: `evals/reports/harbor/roder-tbench-gemini35-flash-validation.md`
- Baseline validation:
  `evals/reports/harbor/roder-tbench-gemini35-flash-validation-baseline.md`

Treat the baseline as a harness/provider health check, not a quality ceiling.
A clean run means no setup, provider API, artifact, timeout, or verifier
harness failures. Reward-0 task failures are still normal scored task failures.

## Run Recipe

Run readiness and image preflight first:

```sh
python3 evals/harbor/validate_harbor_readiness.py \
  --config evals/harbor/tbench-gemini35-flash-validation.json \
  --require-prebuilt

python3 evals/harbor/preflight_tbench_images.py \
  --config evals/harbor/tbench-gemini35-flash-validation.json \
  --offline \
  --manifest /tmp/roder-gemini35-flash-validation-images.json
```

Run Harbor with a Gemini API key available in one of:
`GEMINI_API_TOKEN`, `GEMINI_API_KEY`, `GOOGLE_API_KEY`,
`GOOGLE_GENAI_API_KEY`, or `GOOGLE_AI_API_KEY`.

```sh
PYTHONPATH="$PWD/evals/harbor${PYTHONPATH:+:$PYTHONPATH}" \
  harbor run --config evals/harbor/tbench-gemini35-flash-validation.json
```

If the configured job directory already exists, either move it aside or remove
it before rerunning. The job directory is ignored by git.

```sh
rm -rf evals/harbor/jobs/roder-tbench-gemini35-flash-validation
```

Analyze and validate the run:

```sh
python3 evals/harbor/analyze_tbench_run.py \
  evals/harbor/jobs/roder-tbench-gemini35-flash-validation \
  --json evals/reports/harbor/roder-tbench-gemini35-flash-validation-analysis.json \
  --markdown evals/reports/harbor/roder-tbench-gemini35-flash-validation.md \
  --manifest-dir evals/reports/harbor/gemini35-flash-validation-manifests \
  --group-scored-failures

python3 evals/harbor/validate_tbench_analysis.py \
  evals/reports/harbor/roder-tbench-gemini35-flash-validation-analysis.json \
  --baseline evals/harbor/tbench-clean-baseline.json \
  --expected-trials 6 \
  --markdown evals/reports/harbor/roder-tbench-gemini35-flash-validation-baseline.md
```

## Expected Gate

For small validation, require:

- `scored_trials = 6`
- `harbor_n_errors = 0`
- `clean = true`
- no provider API/tool-schema errors
- no missing artifacts

The current expected score is `5/6`. A lower score may still be a clean harness
run, but it should be investigated as a model/task-quality regression.
