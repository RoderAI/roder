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
Before a live smoke, run the same handoff gate without starting Harbor:

```sh
RODER_HARBOR_DRY_RUN=1 ./evals/harbor/run-roder-tbench-smoke.sh
```

Dry-run mode runs or validates the pre-eval summary for `tbench-smoke.json`,
including prebuilt binary, auth, unit-test, TBench diagnostics, and smoke image
preflight evidence. It exits before job replacement or `harbor run`.
It also writes and validates a smoke launch plan at
`evals/reports/harbor/roder-tbench-smoke-launch-plan.json` by default. Set
`RODER_HARBOR_LAUNCH_PLAN` to override that path. The plan binds the smoke
config hash, pre-eval summary hash, injected prebuilt binary, auth JSON shape,
deadline policy, image preflight evidence, harness file digests, and job-dir
blocking state before Harbor can start.
Live smoke runs preserve an existing `evals/harbor/jobs/roder-tbench-smoke`
directory unless `RODER_HARBOR_REPLACE_JOB=1` is set.

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

Validate the analyzer output against the checked-in clean-run baseline before
spending follow-up runs:

```sh
python3 evals/harbor/validate_tbench_analysis.py \
  evals/reports/harbor/roder-tbench-full-gpt55-medium-analysis.json \
  --baseline evals/harbor/tbench-clean-baseline.json \
  --markdown evals/reports/harbor/roder-tbench-full-gpt55-medium-baseline.md
```

Before widening a generated campaign, compare the latest full baseline against
historical focused reruns and the current campaign manifest:

```sh
python3 evals/harbor/suggest_tbench_campaign.py \
  --baseline evals/reports/harbor/roder-tbench-full-gpt55-medium-deadline-reliability-analysis.json \
  --campaign-manifest evals/reports/harbor/campaigns/validated-conversions/validated-conversions-manifest.json \
  --evidence evals/reports/harbor/roder-tbench-full-gpt55-medium-analysis.json \
  --evidence evals/reports/harbor/roder-tbench-full-gpt55-medium-strict-analysis.json \
  --evidence evals/reports/harbor/roder-tbench-deadline-exec-reliability-rerun-analysis.json \
  --evidence evals/reports/harbor/roder-tbench-remaining-failures-gpt55-xhigh-analysis.json \
  --evidence evals/reports/harbor/roder-tbench-remaining-failures-gpt55-xhigh-plan-first-v2-analysis.json
```

The suggester is read-only. It reports baseline failures that have a historical
pass in the evidence set but are not already included in the campaign.

Generate those missing historical wins as their own reviewed campaign:

```sh
python3 evals/harbor/generate_tbench_campaign.py \
  --campaign historical-wins \
  --output-dir evals/reports/harbor/campaigns/historical-wins
```

Before launching either route set, prove the two manifests combine without task
overlap and inspect the combined projection:

```sh
python3 evals/harbor/summarize_tbench_campaigns.py \
  evals/reports/harbor/campaigns/validated-conversions/validated-conversions-manifest.json \
  evals/reports/harbor/campaigns/historical-wins/historical-wins-manifest.json \
  --preset validated-plus-historical \
  --json evals/reports/harbor/campaigns/validated-plus-historical-summary.json
```

That combination should report 18 unique tasks and a projected 68/89 if every
route reproduces. The preset enables the overlap, campaign/route skeleton,
count, projection, historical-win task, and route-owner checks so stale campaign
math or missing historical-win routes block the handoff. The saved JSON includes
a `validation` block with the preset name, status, expectation fields, and any
blocking issues, plus a SHA-256 binding for each input campaign manifest.

For a targeted subset or campaign route, add `--expected-trials` with the route
task count. That keeps the same harness/provider/runtime blockers while
scaling the full-run `task_dirs` and `scored_trials` requirements to the subset:

```sh
python3 evals/harbor/validate_tbench_analysis.py \
  evals/reports/harbor/campaigns/verifier-contract/near-misses-analysis.json \
  --baseline evals/harbor/tbench-clean-baseline.json \
  --expected-trials 7
```

The baseline gate follows the phase-50 eval style: unknown, setup/artifact,
provider/runtime, and Harbor harness errors block; reward-0 scored failures stay in the score backlog.
The pre-eval diagnostic wrapper can run the same gate and store the report with
the other local diagnostics:

```sh
evals/harbor/run-roder-pre-eval-diagnostics.sh \
  --campaign-summary evals/reports/harbor/campaigns/validated-plus-historical-summary.json \
  --analysis evals/reports/harbor/roder-tbench-full-gpt55-medium-analysis.json
```

The wrapper validates the local TBench diagnostic `eval-run.json` with
`validate_pre_eval_tbench_diagnostics.py`, requiring pass outcomes, completed
verification, zero unknown reliability errors, and exactly the nine fixed diagnostic
fixtures before it writes an `ok` summary. The fixed local suite covers exact
output files, JSON-array serialization, bounded sequences, numeric tolerances,
output-directory hygiene, visible verifier constants, artifact-checkpoint
task ledgers, service-target sanity, and verifier dependency parity. Fixtures with local verifier commands must also report expected
required and completed command-check coverage, and the artifact-checkpoint
fixture must report the expected task-ledger update/completion metrics. The
summary's `tbenchDiagnostics` section records the fixture IDs,
`missingExpectedFixtures`, `unexpectedFixtures`, `duplicateFixtures`, `missingCommandChecks`,
`verifierCommandChecksRequired`, `verifierCommandChecksCompleted`,
`missingTaskLedgerCheckpoints`, and `taskLedgerCheckpointFixtures`, so direct
summary checks enforce the same fixed suite contract as the validator.
`validate_pre_eval_summary.py` also rejects reused summaries with non-empty
`missingCommandChecks`, command-check totals or fixture entries that do not
match the fixed contract, missing/stale fixture fields, missing
aggregate count fields, or `fixtures`/`passed`/`failed` counts that no longer
prove every diagnostic fixture passed.

Each successful wrapper run writes `pre-eval-summary.json` in its output
directory. Treat that file as the handoff artifact before a live run: it records
the git head, dirty-path count, bounded dirty-path list, enabled options,
image-preflight intent, check statuses, Harbor config invariants, and report
paths. When `--campaign-summary` is supplied, the `campaignSummary` check records
the combined campaign preset, validation status, expected campaign/route/task
set, duplicate-task evidence, projected pass count, unique task count, blocking issues, and input
manifest SHA-256 bindings. For the `validated-plus-historical` handoff, the
writer and downstream validators enforce the expected route skeleton, 18 unique
tasks, zero duplicate tasks, and 68 projected passes. The `harborConfigs` check
records concurrency, deadlines, speed-policy state, task-ledger state,
environment cleanup, and prebuilt/source-upload settings for each checked-in
config, plus the exact per-config readiness issues from
`validate_harbor_readiness.py`. Live config verification rejects older summaries
that do not include the full current checked-in Harbor config set. Pass
`--config PATH` to `run-roder-pre-eval-diagnostics.sh` for generated route
configs so the same readiness and summary artifact also attest those files. When
image preflight is enabled, the wrapper automatically adds the `--image-config`
file to that readiness and summary attestation, and the validator rejects
handoffs where the image config is missing from `harborConfigs`. The top-level `status` is `ok` only when every
check is clean; otherwise
`blockedChecks` names the gate that must be fixed before a live run. The wrapper
exits non-zero after writing a blocked summary, including early readiness/auth,
analysis-input, and image-preflight setup failures when enough context is
available. The `harborHarness` section records SHA-256 digests for the Harbor
adapter, wrapper, validator, and shared helper files used during launch, and the
summary validator rejects a handoff that omits or fails that attestation. Live
file verification also rejects older summaries that do not include the full
current harness file set. Blocked summaries include
the failed wrapper step and exit code when the shell trap catches the failure. Analysis-baseline summaries include
`blockedMetrics` and the validator metric snapshot, so a failed clean-run gate is
actionable from `pre-eval-summary.json`. It also records the prebuilt Roder binary
path, executable bit, size, SHA-256, modification time, and `file(1)` type so a
live run can be tied back to the exact injected artifact.
When `--require-prebuilt` is set, the readiness gate rejects executable files
that are not Linux x86-64 ELF binaries. The summary records the same
`linuxX8664Elf` check and blocks if a required prebuilt is the wrong
architecture or file type.
When `--require-auth` is set, the readiness gate checks the Codex auth JSON that
will be uploaded into the task container. The summary records file metadata and
JSON field names only, not token values.
When `--preflight-images` is set, the wrapper runs
`preflight_tbench_images.py` and includes the image manifest summary. `present`,
`missing`, and `unresolved` are task-level counts; `uniqueImages` records the
separate Docker image count. A clean preflight requires `present` to cover every
task; routes that share a Docker image should reduce `uniqueImages`, not
`present`. For configs
without explicit `task_names`, the wrapper lets the preflight read Harbor
registry metadata to establish full task scope without pulling Docker images.
Use `--offline-images` only for route configs that already enumerate task names.
Failed image preflight summaries include `selectionErrors` and `blockedTasks` with the
affected task, status, image, and image source. When image preflight is
required, the summary validator rejects a handoff whose image manifest was
produced for a different Harbor config or whose image config was not included in
the live config attestation. Smoke and full wrappers additionally require the
image-preflight config to match the wrapper config, so a full-run summary cannot
stand in for smoke image evidence. Add `--pull-images` only when the run is
allowed to fetch missing task images.

Validate a saved handoff summary before a live run:

```sh
python3 evals/harbor/validate_pre_eval_summary.py \
  /tmp/roder-pre-eval/pre-eval-summary.json \
  --require-prebuilt --require-auth --require-tests \
  --require-image-preflight --verify-harbor-configs \
  --verify-harness-files --verify-prebuilt-binary \
  --verify-auth-file --require-campaign-summary \
  --campaign-summary /path/to/validated-plus-historical-summary.json \
  --max-age-seconds 7200
```

The guarded full-run wrapper enforces this gate before it starts Harbor. If
`RODER_HARBOR_PRE_EVAL_SUMMARY` points at an existing summary, that summary is
validated. If it is unset, the wrapper runs
`run-roder-pre-eval-diagnostics.sh --require-prebuilt --require-auth
--preflight-images` into
`evals/reports/pre-eval-diagnostics/full-run-latest/`, then validates the
fresh summary. The wrapper also passes `--verify-harness-files` so a reused
handoff summary cannot hide adapter drift, and `--verify-harbor-configs` so
Harbor config drift is caught at the same boundary.
It also verifies the prebuilt Roder binary SHA and re-reads auth JSON shape
without recording a secret hash.
`RODER_HARBOR_PREFLIGHT_PULL=1` is passed through to the
diagnostic image preflight. Set `RODER_HARBOR_PRE_EVAL_ANALYSIS` to include a
previous analyzer JSON or job directory in the gate, and set
`RODER_HARBOR_PRE_EVAL_CAMPAIGN_SUMMARY` to require the combined campaign
handoff summary in both pre-eval and launch-plan validation. Set
`RODER_HARBOR_PRE_EVAL_MAX_AGE_SECONDS` to tighten or relax the default
two-hour freshness window.

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

To turn the current score-plan evidence into executable Harbor configs, generate
the validated-conversions campaign:

```sh
python3 evals/harbor/generate_tbench_campaign.py \
  --output-dir evals/reports/harbor/campaigns/validated-conversions
```

This writes a manifest with per-route image-preflight and launch-plan paths plus
one config per route. It intentionally avoids a mixed-mode Harbor config because
Harbor applies one agent profile to a run. It also writes an executable guarded run script,
`run-validated-conversions.sh`, beside the manifest. The script validates the
campaign, runs per-route image preflight, revalidates the image manifests, and
then exits unless `RODER_HARBOR_LIVE_TBENCH=1` is set. When live execution is
enabled, it runs or validates the pre-eval summary gate, writes and validates a
ready launch plan for each route after the job-directory guard, then runs
`harbor run`. Dry-run mode writes and validates dry-run route launch plans before
exiting. After each route run, the script analyzes the result with
`analyze_tbench_run.py --require-clean`, writing route JSON, Markdown, and rerun
manifests. It also validates each route analysis
against the clean-run baseline with `--expected-trials` set to that route's task
count, so subset campaigns are blocked by harness regressions but not by the
full-run 89-task minimum. Generated scripts pass each route config through the
pre-eval `--config` option and validate reused summaries with `--require-config`,
so a summary for only the default full and smoke configs cannot launch route
campaigns. When `RODER_HARBOR_PRE_EVAL_CAMPAIGN_SUMMARY` is set, generated
scripts also require that campaign summary in pre-eval generation, summary
validation, launch-plan writing, and launch-plan validation. Live generated scripts
also refuse to replace existing route job
directories unless `RODER_HARBOR_REPLACE_JOB=1` is set, preserving prior route
evidence by default. The generated `route_job_dirs` preservation array must
exactly match the manifest route job directories. The default campaign currently
emits:

- `medium-validated`: medium-reasoning focused conversions that need
  reproducibility.
- `xhigh-validated`: selective `gpt-5.5` xhigh conversions.
- `xhigh-plan-first`: plan-first planning at medium, implementation at xhigh.

Use `--list` to see additional campaigns such as `verifier-contract` and
`environment-target`; validate the manifest before any live route run:

```sh
python3 evals/harbor/validate_tbench_campaign.py \
  evals/reports/harbor/campaigns/validated-conversions/validated-conversions-manifest.json
```

If image preflight manifests have already been written for each route, require
them too:

```sh
python3 evals/harbor/validate_tbench_campaign.py \
  evals/reports/harbor/campaigns/validated-conversions/validated-conversions-manifest.json \
  --require-image-preflight \
  --preflight-dir evals/reports/harbor/campaigns/validated-conversions
```

The validator checks each route manifest against its generated Harbor config,
uses parallelism 4, keeps Docker cleanup disabled, preserves the deterministic
artifact set, records expected route analysis/image-preflight paths, and when
requested, has a clean image-preflight manifest for the same route config. It
also rejects stale or hand-edited run scripts that drop the live-run guard,
route image preflight, pre-eval summary validation, job-directory preservation,
or post-run analysis gates. The script must also contain the exact route config,
job, image-preflight, launch-plan, and analysis paths recorded in the manifest, and its
`harbor run --config`, image-preflight `--config`, pre-eval `--config`, and
summary `--require-config` arguments must match the manifest route set exactly.
Each image-preflight command must bind the route config to that route's exact
image manifest path and must use the generated `preflight_args` array so route
preflight stays offline unless explicit pull mode is enabled.
The generated script must invoke the pre-eval diagnostics with its constructed
`pre_eval_args` array and invoke summary validation with its constructed
`summary_validation_args` array, so route-specific handoff gates cannot be built
and then bypassed.
Each route launch plan must bind the route config, job directory, analysis
outputs, and pre-eval summary into `write_tbench_launch_plan.py`, then validate
that plan with `validate_tbench_launch_plan.py --allow-dry-run` in dry-run mode
or `--require-ready` in live mode. After dry-run launch plans exist, standalone
campaign validation can require them too:

```sh
python3 evals/harbor/validate_tbench_campaign.py \
  evals/reports/harbor/campaigns/validated-conversions/validated-conversions-manifest.json \
  --require-image-preflight --require-launch-plans \
  --allow-dry-run-launch-plans \
  --preflight-dir evals/reports/harbor/campaigns/validated-conversions
```

That gate reads each route's `launchPlan`, checks the plan's route config, job
directory, analysis paths, and image manifest against the campaign manifest, then
reuses `validate_tbench_launch_plan.py` to verify the config hash and image
manifest binding. It also requires all route launch plans in the same campaign
handoff to use the same pre-eval summary path, summary SHA-256, and freshness
window, pre-eval output directory, launch-option requirements, and embedded
campaign-summary binding. Live route scripts run the same campaign-level launch-plan
check without `--allow-dry-run-launch-plans`, so only ready plans pass before the
final campaign handoff.
Each `analyze_tbench_run.py` command must also use the route's exact job
directory, `--require-clean`, JSON output, Markdown output, and rerun-manifest
directory, and each `validate_tbench_analysis.py` command must use the route's
exact analysis JSON and task count with the checked-in
`evals/harbor/tbench-clean-baseline.json` baseline. For each live route, the
commands must execute in order: ready launch-plan write, ready launch-plan
validation, `harbor run`, analyzer, then baseline validation. The
final generated command must revalidate the campaign with both
`--require-image-preflight` and `--require-analysis`, and it must run after every
route baseline validation. With
that gate enabled, route manifests also use the pre-eval manifest row checks,
including task rows, image rows, status counts, image-to-task mapping, and exact
route task-name coverage. With
`--require-analysis`, it also validates each route analysis against
`tbench-clean-baseline.json` using the route's `taskCount` as `--expected-trials`,
so standalone campaign validation enforces the same blocker contract as the run
script. Prefer the generated script for a reviewed handoff:

```sh
evals/reports/harbor/campaigns/validated-conversions/run-validated-conversions.sh
```

For a route campaign handoff, dry-run the pre-eval gate or set the live guard:

```sh
RODER_HARBOR_DRY_RUN=1 evals/reports/harbor/campaigns/validated-conversions/run-validated-conversions.sh
RODER_HARBOR_LIVE_TBENCH=1 evals/reports/harbor/campaigns/validated-conversions/run-validated-conversions.sh
```

Set `RODER_HARBOR_PREFLIGHT_PULL=1` only when the route preflight is allowed to
pull missing Docker images; generated route preflight stays local because route
configs enumerate task names. To run a single generated config by hand, use the
same adapter import path used for other generated reruns:

```sh
PYTHONPATH="$PWD/evals/harbor${PYTHONPATH:+:$PYTHONPATH}" \
  harbor run --config evals/reports/harbor/campaigns/validated-conversions/validated-conversions-xhigh-validated.json
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

By default the wrapper runs the local pre-eval diagnostic loop first and refuses
to start Harbor unless the resulting `pre-eval-summary.json` is fresh, unblocked,
and proves prebuilt binary, auth, unit-test, TBench diagnostic, and image
preflight readiness. To reuse an already generated summary, set
`RODER_HARBOR_PRE_EVAL_SUMMARY=/path/to/pre-eval-summary.json`. To require the
validated campaign handoff for a routed score run, also set
`RODER_HARBOR_PRE_EVAL_CAMPAIGN_SUMMARY=/path/to/validated-plus-historical-summary.json`;
when the wrapper runs the diagnostic loop itself, it passes that path through as
`--campaign-summary`.

To validate the launch gate without starting Harbor, run:

```sh
RODER_HARBOR_DRY_RUN=1 \
  RODER_HARBOR_PRE_EVAL_SUMMARY=/path/to/pre-eval-summary.json \
  RODER_HARBOR_PRE_EVAL_CAMPAIGN_SUMMARY=/path/to/validated-plus-historical-summary.json \
  RODER_HARBOR_LAUNCH_PLAN=/tmp/roder-tbench-launch-plan.json \
  ./evals/harbor/run-roder-tbench-full.sh
```

Dry-run mode does not require `RODER_HARBOR_LIVE_TBENCH=1`, does not check for
the Harbor executable, and exits before job replacement or `harbor run`. The
wrapper writes a JSON launch plan by default to
`evals/reports/harbor/roder-tbench-full-gpt55-medium-launch-plan.json`; set
`RODER_HARBOR_LAUNCH_PLAN` to override that path. The plan includes the
validated pre-eval summary, Harbor config, job directory, analysis outputs, and
whether image preflight or analysis gates are required. The same launch-plan
path is written for a live wrapper run after the pre-eval summary gate and
before job replacement or `harbor run`. Use `launchStatus` (`dry_run`, `ready`,
or `blocked`) and `blockedReasons` as the high-level machine gate. The plan also
records `maxPreEvalAgeSeconds`, so standalone launch-plan validation can enforce
the same summary freshness limit used when the plan was written. It also records
`jobDirExists`, `jobDirBlocksLaunch`, `blockedBeforeHarbor`, and `wouldRunHarbor`,
so an existing job directory is visible before the wrapper attempts the live run. The `preEvalSummaryStatus` object repeats the validated
summary status, blocked checks, generation time, and git head when available.
Launch-plan validation requires this embedded summary status to be `ok` with no
blocked checks. `preEvalSummarySha256` binds the plan to the exact summary file;
pass `--verify-pre-eval-summary` to verify the referenced file still matches.
When a campaign summary is required, the plan records `requireCampaignSummary`
and repeats the `campaignSummary` check from the pre-eval summary; launch-plan
validation rejects missing, mismatched, stale, or wrong-count campaign-summary
copies. The wrapper also passes `RODER_HARBOR_PRE_EVAL_CAMPAIGN_SUMMARY` into
`validate_pre_eval_summary.py --campaign-summary`, so a reused summary generated
from a different campaign handoff is rejected before the launch plan is written.
`harborConfigSha256` binds the plan to the exact Harbor config, and
`preEvalHarborConfigSha256` repeats the hash recorded by the pre-eval readiness
summary for that same config path. Pass `--verify-harbor-config` to verify that
the config file still matches both hashes and that readiness was checked against
the same config being launched. The `prebuiltBinary` object repeats the
prebuilt Roder path and SHA-256 from the pre-eval summary; pass
`--verify-prebuilt-binary` to verify that the injected binary has not changed
since the diagnostic loop. The
`authFile` object repeats the auth file path and JSON-shape metadata; pass
`--verify-auth-file` to re-read the file and ensure it is still valid JSON
without recording a secret-content hash. `--verify-pre-eval-summary` also
re-runs the full summary gate against the referenced handoff, and
`--max-pre-eval-age-seconds` applies the same freshness limit during launch-plan
validation. When image preflight is required, the `imagePreflight` object records
the manifest, config, and clean-count details, and launch-plan validation rejects
blocked image summaries, a manifest produced for a different Harbor config, or a
manifest whose task count does not match the referenced Harbor config.
The `harborHarness` object records SHA-256 digests for the Harbor
adapter, wrapper, validator, and shared helper files used at launch; pass
`--verify-harness-files` to reject adapter drift or incomplete older harness
snapshots between diagnostics and `harbor run`. The full-run wrapper
applies all five verifications automatically.

Validate the launch plan with:

```sh
python3 evals/harbor/validate_tbench_launch_plan.py \
  evals/reports/harbor/roder-tbench-full-gpt55-medium-launch-plan.json \
  --allow-dry-run --verify-pre-eval-summary --verify-harbor-config \
  --verify-prebuilt-binary --verify-auth-file --verify-harness-files \
  --require-campaign-summary --max-pre-eval-age-seconds 7200
```

Use `--require-ready` for a live launch plan that must be clear to reach
`harbor run`; blocked plans print the `blockedReasons` list. The full-run
wrapper runs the same validator after writing the launch plan: dry runs validate
with `--allow-dry-run`, while live runs must pass `--require-ready` before image
preflight, job replacement, or `harbor run`. Add
`--require-image-preflight` when a ready plan must prove that image preflight is
enabled; the wrapper applies that flag automatically unless
`RODER_HARBOR_SKIP_PREFLIGHT=1`.

The checked-in full config runs four Terminal-Bench trials at a time through
Harbor's local orchestrator.

## Gemini 3.5 Flash Validation

`evals/harbor/tbench-gemini35-flash-validation.json` is the native-Gemini
validation set for `gemini/gemini-3.5-flash`. It uses six Terminal-Bench 2.0
tasks with existing GPT-5.5 pass evidence and clean Harbor verifier behavior:

- Medium-run passes:
  `configure-git-webserver`, `headless-terminal`, `regex-log`,
  `sqlite-db-truncate`.
- Xhigh rerun passes:
  `kv-store-grpc`, `polyglot-c-py`.

`db-wal-recovery` and `query-optimize` are intentionally excluded from this
small validation set because they did not produce clean Harbor scoring artifacts
during Gemini harness validation.

The current clean baseline from May 26, 2026 is `5/6` (`83.3%`) with zero
Harbor errors: five passes and one scored failure on `polyglot-c-py`.

The full small-subset runbook is in
`evals/harbor/GEMINI35_FLASH_VALIDATION.md`.

Run the local non-live gates before using it:

```sh
python3 evals/harbor/preflight_tbench_images.py \
  --config evals/harbor/tbench-gemini35-flash-validation.json \
  --offline \
  --manifest /tmp/roder-gemini35-flash-validation-images.json

python3 evals/harbor/validate_harbor_readiness.py \
  --config evals/harbor/tbench-gemini35-flash-validation.json
```

For a live Gemini validation run, provide a Gemini API key through
`GEMINI_API_TOKEN`, `GEMINI_API_KEY`, `GOOGLE_API_KEY`,
`GOOGLE_GENAI_API_KEY`, or `GOOGLE_AI_API_KEY`, then run Harbor explicitly:

```sh
PYTHONPATH="$PWD/evals/harbor${PYTHONPATH:+:$PYTHONPATH}" \
  harbor run --config evals/harbor/tbench-gemini35-flash-validation.json
```

Analyze the result with `--expected-trials 6` so the same clean-run baseline
blocks harness/provider errors without requiring a full 89-task run.

## Auth

For the default `codex/gpt-5.5` model, the adapter copies
`~/.roder/auth/codex.json` into the container's isolated `RODER_CONFIG_DIR`.
Override this with:

```sh
export RODER_HARBOR_AUTH_FILE="$HOME/.gode/auth/codex.json"
```

## Source

The checked-in smoke and full configs inject only the prebuilt Linux `roder`
binary. They set `include_prebuilt_binary=true` and `include_local_source=false`
so a missing binary fails setup instead of uploading and compiling the source
tree inside each Terminal-Bench task container.

For local adapter debugging only, set `include_local_source=true` in a temporary
config. The source fallback uploads a tarball of the current repo checkout,
excluding `.git`, `target`, and generated eval output. Override the source root
with:

```sh
export RODER_HARBOR_SOURCE_DIR=/Users/pz/w/gode
```

For remote-only setup, set `include_local_source=false` in the agent kwargs and
provide:

```sh
export RODER_HARBOR_GIT_URL=https://github.com/PandelisZ/gode.git
export RODER_HARBOR_GIT_REF=main
```
