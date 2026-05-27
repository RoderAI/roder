# Roder Evals

Roder evals run local fixture suites through the Rust runtime and write two files:

- `eval-run.json`: structured run, fixture, trajectory, metric, and failure data.
- `eval-report.md`: bounded human-readable summary with failure groups by tool, model, and failure class.

Normal contributor runs are offline and use the deterministic `mock/mock` provider:

```sh
RODER_EVAL_OUTPUT_DIR=/tmp/roder-evals roder eval run evals/fixtures --offline
roder eval list --output-dir /tmp/roder-evals
roder eval report eval-run --output-dir /tmp/roder-evals --max-bytes 65536
```

If `--output-dir` is omitted, the CLI uses `RODER_EVAL_OUTPUT_DIR`, then `evals/reports`.

Run the focused Terminal-Bench diagnostic loop before another Harbor full run:

```sh
RODER_EVAL_OUTPUT_DIR=/tmp/roder-evals-tbench-diagnostics \
  cargo run -p roder-cli -- eval run evals/fixtures/tbench-diagnostics --offline --profile eval
```

This suite covers exact required output files, JSON array serialization, bounded
sequence outputs, numeric tolerance checks, output-directory hygiene, visible
verifier constants, artifact-checkpoint task ledgers, and service-target sanity
checks, plus verifier dependency parity fallback evidence. It is deliberately
smaller than Terminal-Bench: use it to validate harness and verifier-contract
behavior before spending a full Harbor run.

The Harbor pre-eval wrapper runs the diagnostic suite and the focused eval unit
tests, with speed-policy comparison opt-in:

```sh
evals/harbor/run-roder-pre-eval-diagnostics.sh
evals/harbor/run-roder-pre-eval-diagnostics.sh --include-speed
evals/harbor/run-roder-pre-eval-diagnostics.sh --require-prebuilt --require-auth
evals/harbor/run-roder-pre-eval-diagnostics.sh --preflight-images
evals/harbor/run-roder-pre-eval-diagnostics.sh \
  --analysis evals/reports/harbor/roder-tbench-full-gpt55-medium-analysis.json
evals/harbor/run-roder-pre-eval-diagnostics.sh \
  --config evals/reports/harbor/campaigns/validated-conversions/validated-conversions-xhigh-validated.json
```

The one-task Harbor smoke uses the same non-live gate:

```sh
RODER_HARBOR_DRY_RUN=1 evals/harbor/run-roder-tbench-smoke.sh
```

This validates the smoke Harbor config, prebuilt binary, auth file, local TBench
diagnostics, and smoke image preflight before any job replacement or `harbor run`.
The live smoke wrapper also refuses to replace an existing smoke job directory
unless `RODER_HARBOR_REPLACE_JOB=1` is set, preserving prior evidence by default.

The wrapper validates Harbor config readiness before running evals: doubled
local deadlines, speed policy disabled for the deadline experiment, full-run
parallelism, deterministic artifacts, no post-run container cleanup, and ignored
generated output paths. It validates the TBench diagnostic `eval-run.json` with
`validate_pre_eval_tbench_diagnostics.py`, which requires pass outcomes,
completed verification, zero unknown reliability errors, and exactly the nine expected
fixture IDs: `tbench-exact-output-file`, `tbench-json-array-output`,
`tbench-numeric-tolerance-output`, `tbench-output-directory-hygiene`,
`tbench-sequence-output`, `tbench-visible-verifier-contract`, and
`tbench-artifact-checkpoint`, `tbench-service-target-sanity`, and
`tbench-verifier-dependency-parity`. Fixtures with local verifier commands must also
report expected `verifier_command_checks_required` and completed
`verifier_command_checks_completed` coverage, and the artifact-checkpoint fixture
must report the expected task-ledger update/completion metrics. When
`--analysis` is provided, it also validates the prior Harbor analyzer output
against the clean-run baseline and stores the baseline report under the
diagnostic output directory. Every successful wrapper run writes
`pre-eval-summary.json` with the git head, dirty-path count, bounded dirty-path
list, enabled options, check statuses, Harbor config invariants, diagnostic fixture IDs,
`missingExpectedFixtures`, `unexpectedFixtures`, `duplicateFixtures`, `missingCommandChecks`,
`verifierCommandChecksRequired`, `verifierCommandChecksCompleted`,
`missingTaskLedgerCheckpoints`, `taskLedgerCheckpointFixtures`, report paths,
and prebuilt Roder binary metadata including size, SHA-256, file type,
executable status, and `linuxX8664Elf`. The
`harborConfigs` section includes the same per-config readiness issues used by
`validate_harbor_readiness.py`, so the summary gate is not weaker than the
standalone readiness check. Live config verification rejects older summaries
that do not include the full current checked-in Harbor config set. Pass
`--config PATH` for generated route configs so the same readiness and summary
artifact also attest those files. When image preflight is enabled, the wrapper
also attests the `--image-config` file through the same `harborConfigs` evidence
and summary validation requires that config to be present. The `harborHarness` section records SHA-256 digests
for the Harbor adapter, wrapper, validator, and shared helper files used during
launch, and `validate_pre_eval_summary.py` rejects summaries that omit or fail
that attestation. Live file verification also rejects older summaries that do
not include the full current harness file set. It also rejects reused summaries with non-empty
`missingCommandChecks`, command-check totals or fixture entries that do not
match the fixed contract, missing `fixtureIds`/fixture-status fields, a
`fixtureIds` list that omits one of the fixed diagnostic fixtures, includes
unexpected IDs, or repeats IDs, missing aggregate count fields, or
`fixtures`/`passed`/`failed` counts that no longer prove every diagnostic fixture passed. If `--analysis`
blocks on the clean-run baseline, the
`harborAnalysisBaseline` section records `blockedMetrics` and the validator
metric snapshot so the failed gate is visible without opening the separate
validation JSON. The summary has a top-level `status` field (`ok` or `blocked`) and a
`blockedChecks` list so the next operator can gate a live run without scraping
individual sections; the wrapper exits non-zero after writing the summary if
that top-level status is blocked. Early failures also write a blocked summary
when enough context is available, so readiness/auth or analysis-input mistakes
and image-preflight setup mistakes leave a handoff artifact instead of only
terminal output. Blocked summaries include the failed wrapper step and exit code
when the shell trap catches the failure. With
`--require-prebuilt`, readiness also verifies the configured binary is a Linux
x86-64 ELF before Harbor can inject it into task containers. With
`--require-auth`, readiness verifies that the Codex auth JSON Harbor will upload
exists and has the required field shape; the summary records only auth file
metadata, never token contents.
With `--preflight-images`, the wrapper runs the Terminal-Bench Docker image
preflight and records the manifest path plus task-level present/missing/unresolved
counts and the separate unique-image count. For a clean preflight, task-level
`present` must equal `tasks`; shared Docker images are represented by a lower
`uniqueImages` count, not by a lower `present` count.
For configs without explicit `task_names`, the preflight may read Harbor registry
metadata to establish the full task scope, but it will not pull Docker images
unless `--pull-images` is also set. Use `--offline-images` only for route configs
that already enumerate task names.
When image preflight is required, `validate_pre_eval_summary.py` rejects a
handoff whose image manifest was produced for a different Harbor config or whose
`--image-config` was not included in the live config attestation. Failed
image preflight summaries also include `selectionErrors` and `blockedTasks` with
the affected task, status, image, and image source. Wrappers also require the
image-preflight config to match the config being dry-run or launched, so a
full-suite image summary cannot be reused as smoke evidence. Use `--pull-images`
only when intentionally allowing the preflight to pull missing images.

Validate a saved handoff summary before a live run:

```sh
python3 evals/harbor/validate_pre_eval_summary.py \
  /tmp/roder-pre-eval/pre-eval-summary.json \
  --require-prebuilt --require-auth --require-tests \
  --require-image-preflight --verify-harbor-configs \
  --verify-harness-files --verify-prebuilt-binary \
  --verify-auth-file --max-age-seconds 7200
```

The guarded Harbor full-run wrapper now enforces this handoff gate before it
starts Harbor. If `RODER_HARBOR_PRE_EVAL_SUMMARY` is set, the wrapper validates
that file. Otherwise it runs `run-roder-pre-eval-diagnostics.sh` with
`--require-prebuilt`, `--require-auth`, and image preflight, then validates the
freshly written summary. The wrapper also passes `--verify-harness-files` so a
reused handoff summary cannot hide adapter drift, and
`--verify-harbor-configs` so Harbor config drift is caught at the same boundary.
It also verifies the prebuilt Roder binary SHA and re-reads auth JSON shape
without recording a secret hash.
Set
`RODER_HARBOR_PRE_EVAL_ANALYSIS` to include a prior analyzer JSON or job
directory in the same gate.
Generated route scripts additionally validate reused summaries with
`--require-config` for every route config, so a summary for only the default full
and smoke configs cannot launch a campaign route.

Run the same gate without starting Harbor:

```sh
RODER_HARBOR_DRY_RUN=1 \
  RODER_HARBOR_PRE_EVAL_SUMMARY=/tmp/roder-pre-eval/pre-eval-summary.json \
  RODER_HARBOR_LAUNCH_PLAN=/tmp/roder-pre-eval/launch-plan.json \
  evals/harbor/run-roder-tbench-full.sh
```

In dry-run mode, the wrapper does not require `RODER_HARBOR_LIVE_TBENCH=1` and
exits before job replacement, image preflight reruns, or `harbor run`. It writes
a JSON launch plan by default to
`evals/reports/harbor/roder-tbench-full-gpt55-medium-launch-plan.json`; set
`RODER_HARBOR_LAUNCH_PLAN` to override that path. The plan records the
validated summary path, Harbor config, job directory, and required gates.
The same variable can be set for a live wrapper invocation; the plan is written
after the pre-eval summary gate and before job replacement or `harbor run`.
Use `launchStatus` (`dry_run`, `ready`, or `blocked`) and `blockedReasons` in
that JSON as the high-level machine gate. The plan records
`maxPreEvalAgeSeconds`, so standalone launch-plan validation can enforce the
same summary freshness limit used when the plan was written. The lower-level `jobDirExists`,
`jobDirBlocksLaunch`, `blockedBeforeHarbor`, and `wouldRunHarbor` fields explain
existing-output-directory blocks before a live run attempt. The
`preEvalSummaryStatus` object repeats the validated summary status, blocked
checks, generation time, and git head when available. Launch-plan validation
requires this embedded summary status to be `ok` with no blocked checks.
`preEvalSummarySha256` binds the plan to the exact summary file; pass
`--verify-pre-eval-summary` to verify the referenced file still matches.
`harborConfigSha256` binds the plan to the exact Harbor config, and
`preEvalHarborConfigSha256` repeats the hash recorded by the pre-eval readiness
summary for that same config path. Pass `--verify-harbor-config` to verify that
the config file still matches both hashes and that readiness was checked against
the same config being launched. The `prebuiltBinary` object repeats the prebuilt Roder
path and SHA-256 from the pre-eval summary; pass `--verify-prebuilt-binary` to
verify that the injected binary has not changed since the diagnostic loop.
The `authFile` object repeats the auth file path and JSON-shape metadata; pass
`--verify-auth-file` to re-read the file and ensure it is still valid JSON
without recording a secret-content hash. `--verify-pre-eval-summary` also
re-runs the full summary gate against the referenced handoff, and
`--max-pre-eval-age-seconds` applies the same freshness limit during launch-plan
validation. When image preflight is required, the
`imagePreflight` object records the manifest status and config path, and
launch-plan validation rejects a manifest produced for a different Harbor
config. Manifest validation also checks task rows, image rows, status counts,
image-to-task mapping, and task count against the referenced Harbor config, so
stale, partial, or internally inconsistent preflight evidence cannot pass the
handoff gate. The `harborHarness` object records SHA-256 digests for the Harbor
adapter, wrapper, validator, and shared helper files used at launch; pass
`--verify-harness-files` to reject adapter drift or incomplete older harness
snapshots between diagnostics and `harbor run`. The full-run wrapper
applies all five verifications automatically.

Validate a launch plan with:

```sh
python3 evals/harbor/validate_tbench_launch_plan.py \
  evals/reports/harbor/roder-tbench-full-gpt55-medium-launch-plan.json \
  --allow-dry-run --verify-pre-eval-summary --verify-harbor-config \
  --verify-prebuilt-binary --verify-auth-file --verify-harness-files \
  --max-pre-eval-age-seconds 7200
```

Use `--require-ready` for a live launch plan that must be clear to reach
`harbor run`. The full-run wrapper runs this validator itself: dry runs validate
with `--allow-dry-run`, while live runs must pass `--require-ready` before image
preflight, job replacement, or `harbor run`. Add
`--require-image-preflight` when a ready plan must prove that image preflight is
enabled; the wrapper applies that flag automatically unless
`RODER_HARBOR_SKIP_PREFLIGHT=1`.

After a Harbor full run, validate the analyzer output against the checked-in
clean-run baseline:

```sh
python3 evals/harbor/validate_tbench_analysis.py \
  evals/reports/harbor/roder-tbench-full-gpt55-medium-analysis.json \
  --baseline evals/harbor/tbench-clean-baseline.json
```

For targeted route campaigns, pass the route task count so the same baseline
blocks harness/provider/runtime regressions without requiring all 89 full-run
trials:

```sh
python3 evals/harbor/validate_tbench_analysis.py \
  evals/reports/harbor/campaigns/verifier-contract/near-misses-analysis.json \
  --baseline evals/harbor/tbench-clean-baseline.json \
  --expected-trials 7
```

This is the Harbor analogue of the reliability baseline gate: harness,
artifact, unknown-error, and provider/runtime regressions block the next
iteration while scored reward-0 tasks remain a score-improvement backlog.
Generated campaign scripts run this subset-aware baseline check after every
route analysis, and `validate_tbench_campaign.py --require-analysis` applies the
same check during standalone campaign validation. Live generated campaign
scripts refuse to replace existing route job directories unless
`RODER_HARBOR_REPLACE_JOB=1` is set, matching the full and smoke wrapper
evidence-preservation default. The generated `route_job_dirs` preservation array
must exactly match the manifest route job directories.
For generated campaigns, `validate_tbench_campaign.py --require-image-preflight`
also checks exact route task-name coverage so a stale same-sized image manifest
cannot be reused for a different route slice. It also inspects the generated run
script and rejects handoff scripts that drop the live-run guard, image preflight,
pre-eval summary validation, job-directory preservation, or post-run analysis
gates. The script must also contain the exact route config, job,
image-preflight, and analysis paths recorded in the manifest, and its
`harbor run --config`, image-preflight `--config`, pre-eval `--config`, and
summary `--require-config` arguments must match the manifest route set exactly.
Each image-preflight command must bind the route config to that route's exact
image manifest path and must use the generated `preflight_args` array so route
preflight stays offline unless explicit pull mode is enabled.
The generated script must invoke the pre-eval diagnostics with its constructed
`pre_eval_args` array and invoke summary validation with its constructed
`summary_validation_args` array, so the validated route set cannot be assembled
and then skipped.
The script must also end with a campaign validation command that requires both
image preflight and route analysis evidence.
Each `analyze_tbench_run.py` command must also use the route's exact job
directory, `--require-clean`, JSON output, Markdown output, and rerun-manifest
directory, and each `validate_tbench_analysis.py` command must use the route's
exact analysis JSON and task count with the checked-in
`evals/harbor/tbench-clean-baseline.json` baseline. For each route, the commands
must execute in order: `harbor run`, analyzer, then baseline validation.
The final campaign validation that requires analysis must run after every route
baseline validation.

Live-provider mode is guarded so it cannot run by accident. A live run must set `RODER_EVAL_LIVE_PROVIDER=1` and pass explicit `--provider` and `--model` values. This phase wires the guard and the mock path:

```sh
RODER_EVAL_LIVE_PROVIDER=1 roder eval run evals/fixtures --provider mock --model mock
```

App-server clients can inspect recent reports without arbitrary file reads:

```json
{ "jsonrpc": "2.0", "id": 1, "method": "eval/reports/list", "params": { "limit": 20 } }
```

```json
{ "jsonrpc": "2.0", "id": 2, "method": "eval/report/read", "params": { "reportId": "eval-run", "maxBytes": 65536 } }
```

The app-server only resolves report ids returned by `eval/reports/list` under the workspace `evals/reports` directory.
