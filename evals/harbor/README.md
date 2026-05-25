# Roder Harbor Harness

This directory contains a Harbor custom agent for running `roder` against
Terminal-Bench tasks.

The adapter uploads a prebuilt Linux `roder` binary into Harbor's Docker task
environment, writes an isolated config/auth directory, and runs one
Terminal-Bench instruction through `roder exec --json --profile eval --mode
bypass --skip-git-repo-check --task-ledger-required`. The prebuilt binary is
the normal path. Source upload/build remains available as a slower fallback for
debugging only.

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
- `roder-run-summary.json`: structured provider, timeout, exit-status, elapsed
  time, artifact-size, and last-event metadata
- `roder-plan.md`, `roder-plan-events.jsonl`, `roder-plan-stderr.txt`, and
  `roder-plan-last-message.txt`: populated when plan-first mode is enabled

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

Compare two analyzer JSON files, or two Harbor job directories, after a rerun:

```sh
python3 evals/harbor/compare_tbench_runs.py \
  evals/reports/harbor/roder-tbench-full-gpt55-medium-strict-analysis.json \
  evals/reports/harbor/roder-tbench-full-gpt55-medium-deadline-reliability-analysis.json \
  --json evals/reports/harbor/strict-vs-deadline-reliability-comparison.json \
  --markdown evals/reports/harbor/strict-vs-deadline-reliability-comparison.md
```

The comparison report lists pass/fail flips, class-only changes, class-count
deltas, missing tasks, and score movement. Use it after every targeted or full
rerun before deciding whether a harness change helped.

Focused rerun notes live in
[`RODER_HARNESS_TARGETED_RERUNS.md`](RODER_HARNESS_TARGETED_RERUNS.md).

The checked-in configs set a Roder soft timeout before Harbor's hard
`override_timeout_sec`. They also pass Roder an internal eval deadline before
that soft timeout. Deadline-aware tools can then stop long commands with enough
time for the agent to observe the timeout. When the eval deadline reserve is
reached, including while a model stream is still active, Roder injects a
finalization prompt, disables further tool calls for that model request, and
asks for the final answer from the current workspace state. If the external
soft timeout still fires, the adapter interrupts
`roder exec`, keeps the partial event/stderr artifacts, exits the agent command
successfully, and lets Terminal-Bench score the workspace state. This prevents a
task that used the full agent window from becoming a Harbor `AgentTimeoutError`;
it will appear as a normal scored pass/fail, with a `soft_timeout` class in the
analysis for traceability.

The full GPT-5.5 config also sets `speed_policy_enabled: false` in the agent
kwargs. Roder's eval runtime can otherwise change reasoning effort by phase, so
this explicit setting keeps the benchmark at the requested `medium` reasoning
level for every model call. The checked-in task window is currently doubled to a
1800-second Harbor hard timeout, 1780-second adapter soft timeout, and
1740-second internal eval deadline; inference-speed work should be measured as a
separate experiment.

The configs pass `task_ledger_required: true` to `roder exec`. In eval profile
this requires the model to maintain the built-in `task_ledger.update` ledger
before risky work, with completion evidence for finished subtasks. The intent is
to improve exact-contract follow-through on Terminal-Bench tasks without
changing normal interactive turns.

The configs raise `reliability_max_consecutive_tool_failures` for eval runs.
The broader per-turn failure cap already prevents runaway loops; the higher
consecutive cap keeps one repair loop from ending the whole task after five
failed probes.

The adapter can also write provider retry settings into the generated Roder
config via kwargs or environment variables:
`reliability_provider_retry_max_attempts`,
`reliability_provider_retry_initial_backoff_ms`,
`reliability_provider_retry_backoff_factor`,
`reliability_provider_retry_status_codes`, and
`reliability_retry_empty_provider_body`. Streaming response failures are
classified separately by the analyzer as `provider_stream_decode_error` or
`provider_stream_incomplete`. In eval mode, Roder retries known transient stream
decode/incomplete failures only before executing tool calls from that failed
stream, and emits a `reliability.retry` event when it does.

The checked-in configs do not override per-task CPU, memory, or storage limits.
Harbor warns that resource overrides can make a Terminal-Bench run unsuitable
for leaderboard comparison.

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
  --eval-deadline-sec 20 \
  --output-config evals/reports/harbor/roder-tbench-soft-timeout-debug.json
```

For plan-first reruns, ask the adapter to run a planning turn first, then
resume the same Roder thread for implementation:

```sh
python3 evals/harbor/rerun_tbench_subset.py \
  --source-job evals/harbor/jobs/roder-tbench-remaining-failures-gpt55-xhigh \
  --base-config evals/harbor/tbench-full-gpt55-medium.json \
  --class scored_fail \
  --reasoning xhigh \
  --plan-first \
  --plan-first-reasoning medium \
  --plan-first-soft-timeout-sec 360 \
  --timeout-sec 2400 \
  --soft-timeout-sec 2000 \
  --eval-deadline-sec 1960 \
  --job-name roder-tbench-remaining-failures-gpt55-xhigh-plan-first \
  --output-config evals/reports/harbor/roder-tbench-remaining-failures-gpt55-xhigh-plan-first.json
```

Plan-first mode is a Harbor adapter mode, not Roder's read-only `plan` policy
mode. The planning turn defaults to the same policy mode as the implementation
turn so it can inspect local task files; it is constrained by prompt and a short
planning soft timeout. Use `--plan-first-reasoning` to keep the planning turn
cheaper while leaving the implementation turn at the requested reasoning level.

Generated jobs, reports, manifests, and binaries are ignored by git under
`evals/harbor/jobs/`, `evals/harbor/artifacts/`, and `evals/reports/`.

When running a generated config outside `evals/harbor`, include the adapter
directory on `PYTHONPATH` so Harbor can import `roder_harbor_agent`:

```sh
PYTHONPATH="$PWD/evals/harbor${PYTHONPATH:+:$PYTHONPATH}" \
  harbor run --config evals/reports/harbor/roder-tbench-soft-timeout-debug.json
```

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
