# Roder Harbor Harness

This directory contains a Harbor custom agent for running `roder` against
Terminal-Bench tasks.

The adapter uploads a prebuilt Linux `roder` binary into Harbor's Docker task
environment, writes an isolated config/auth directory, and runs one
Terminal-Bench instruction through `roder exec --json --profile eval --mode
bypass --skip-git-repo-check`. The prebuilt binary is the normal path. Source
upload/build remains available as a slower fallback for debugging only.

## Smoke Run

```sh
export PATH="$HOME/.local/bin:$PATH"
export RODER_HARBOR_LIVE_TBENCH=1
./evals/harbor/run-roder-tbench-smoke.sh
```

The smoke config runs the `break-filter-js-from-html` task from
`terminal-bench@2.0` with one attempt and writes Harbor results under
`evals/harbor/jobs/`.

The smoke script builds a reusable Linux binary first when
`evals/harbor/artifacts/roder-linux-amd64` is missing. To provide your own
binary:

```sh
export RODER_HARBOR_PREBUILT_BINARY=/path/to/linux/roder
```

By default the smoke script also runs offline image preflight and a clean-run
analysis. If the task cache is intentionally empty, set
`RODER_HARBOR_SKIP_PREFLIGHT=1` for the first smoke run.

Per-run Roder artifacts are copied by Harbor under the agent logs directory:

- `roder-events.jsonl`: `roder exec --json` event stream
- `roder-last-message.txt`: final assistant text
- `roder-cli.txt`: final assistant text plus stderr diagnostics
- `roder-stderr.txt`: warnings and runtime diagnostics
- `setup-summary.txt`: installer and run-command setup diagnostics

## Full Run Hygiene

Before a full Terminal-Bench run, prove Docker image availability:

```sh
python3 evals/harbor/preflight_tbench_images.py \
  --config evals/harbor/tbench-full-gpt55-medium.json \
  --offline \
  --manifest evals/reports/harbor/roder-tbench-full-gpt55-medium-images.json
```

If images are missing and the run is allowed to use the network, pull them with
explicit opt-in:

```sh
RODER_HARBOR_PREFLIGHT_PULL=1 python3 evals/harbor/preflight_tbench_images.py \
  --config evals/harbor/tbench-full-gpt55-medium.json \
  --pull \
  --manifest evals/reports/harbor/roder-tbench-full-gpt55-medium-images.json
```

The smoke and full-run configs keep Harbor Docker state after a run
(`environment.delete: false`). This preserves pulled task images for later
offline preflight checks and follow-up reruns. Clean up Docker state manually
when you intentionally want to reclaim disk space.

After a run, classify failures before deciding whether the issue is Roder,
Terminal-Bench, Docker, setup, timeout, or artifact capture:

```sh
python3 evals/harbor/analyze_tbench_run.py \
  evals/harbor/jobs/roder-tbench-full-gpt55-medium \
  --json evals/reports/harbor/roder-tbench-full-gpt55-medium-analysis.json \
  --markdown evals/reports/harbor/roder-tbench-full-gpt55-medium.md \
  --manifest-dir evals/reports/harbor/manifests \
  --group-scored-failures
```

`--require-clean` exits non-zero when harness-level failures remain. Reward-0
scored tasks are reported separately from clean-run errors.

The checked-in configs set a Roder soft timeout before Harbor's hard
`override_timeout_sec`. When the soft timeout fires, the adapter interrupts
`roder exec`, keeps the partial event/stderr artifacts, exits the agent command
successfully, and lets Terminal-Bench score the workspace state. This prevents a
task that used the full agent window from becoming a Harbor `AgentTimeoutError`;
it will appear as a normal scored pass/fail, with a `soft_timeout` class in the
analysis for traceability.

Generate a targeted rerun config from any analyzer class:

```sh
python3 evals/harbor/rerun_tbench_subset.py \
  --source-job evals/harbor/jobs/roder-tbench-full-gpt55-medium \
  --class docker_registry_bad_gateway \
  --output-config /tmp/roder-tbench-registry-rerun.json
```

For debugging timeout behavior, generate a one-task subset with an explicit
soft deadline:

```sh
python3 evals/harbor/rerun_tbench_subset.py \
  --source-job evals/harbor/jobs/roder-tbench-full-gpt55-medium \
  --class agent_timeout \
  --task-name break-filter-js-from-html \
  --timeout-sec 90 \
  --soft-timeout-sec 30 \
  --output-config evals/reports/harbor/roder-tbench-soft-timeout-debug.json
```

Generated jobs, reports, manifests, and binaries are ignored by git under
`evals/harbor/jobs/`, `evals/harbor/artifacts/`, and `evals/reports/`.

The guarded full-run wrapper combines the same checks:

```sh
export RODER_HARBOR_LIVE_TBENCH=1
export RODER_HARBOR_PREFLIGHT_PULL=1   # only when images may need pulling
export RODER_HARBOR_REPLACE_JOB=1      # only when replacing an existing job dir
./evals/harbor/run-roder-tbench-full.sh
```

The checked-in full config runs four Terminal-Bench trials at a time through
Harbor's local orchestrator.

## Auth

For the default `codex/gpt-5.5` model, the adapter copies
`~/.roder/auth/codex.json` into the container's isolated `RODER_CONFIG_DIR`.
Override this with:

```sh
export RODER_HARBOR_AUTH_FILE="$HOME/.gode/auth/codex.json"
```

## Source

By default the adapter uploads a tarball of the current repo checkout, excluding
`.git`, `target`, and generated eval output. Override the source root with:

```sh
export RODER_HARBOR_SOURCE_DIR=/Users/pz/w/gode
```

For remote-only setup, set `include_local_source=false` in the agent kwargs and
provide:

```sh
export RODER_HARBOR_GIT_URL=https://github.com/PandelisZ/gode.git
export RODER_HARBOR_GIT_REF=main
```
