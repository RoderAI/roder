# Roder Harbor Harness Issues And Improvement Backlog

This document summarizes issues found from the Terminal-Bench full runs used to
shake down the Roder Harbor harness.

Latest deadline/reliability strict-medium run:

- Job: `evals/harbor/jobs/roder-tbench-full-gpt55-medium-deadline-reliability`
- Config:
  `evals/reports/harbor/roder-tbench-full-gpt55-medium-deadline-reliability.json`
- Report:
  `evals/reports/harbor/roder-tbench-full-gpt55-medium-deadline-reliability-analysis.json`
- Markdown report:
  `evals/reports/harbor/roder-tbench-full-gpt55-medium-deadline-reliability.md`
- Model: `codex/gpt-5.5`
- Reasoning: `medium`
- Speed policy: disabled
- Parallelism: 4 concurrent trials
- Docker task resource overrides: none
- Result: 89 trials, 0 Harbor errors, mean `0.5617977528089888`
- Score split: 50 pass, 39 scored fail
- Soft timeouts: 11 total, 3 pass, 8 fail
- Internal deadline timeouts: 3 trials
- Provider/policy-like blocks: 6 trials
- Roder non-zero exec statuses: 9 trials
- Runtime: 2:45:36

Delta vs latest strict baseline:

- Net score movement: +3 passes.
- Improved tasks: `adaptive-rejection-sampler`, `break-filter-js-from-html`,
  `configure-git-webserver`, `extract-moves-from-video`, `git-multibranch`,
  `mcmc-sampling-stan`, `path-tracing-reverse`, `pypi-server`,
  `pytorch-model-recovery`.
- Regressed tasks: `financial-document-processor`, `fix-code-vulnerability`,
  `git-leak-recovery`, `kv-store-grpc`, `llm-inference-batching-scheduler`,
  `query-optimize`.
- Changed failure signals without score movement:
  `install-windows-3.11`, `make-doom-for-mips`, `make-mips-interpreter`,
  `mteb-leaderboard`, `qemu-startup`, `regex-chess`, `sanitize-git-repo`,
  `winning-avg-corewars`.

Latest strict-medium baseline run:

- Job: `evals/harbor/jobs/roder-tbench-full-gpt55-medium-strict`
- Config: `evals/reports/harbor/roder-tbench-full-gpt55-medium-strict.json`
- Report: `evals/reports/harbor/roder-tbench-full-gpt55-medium-strict-analysis.json`
- Model: `codex/gpt-5.5`
- Reasoning: `medium`
- Speed policy: disabled
- Parallelism: 4 concurrent trials
- Docker task resource overrides: none
- Result: 89 trials, 0 Harbor errors, mean `0.5280898876404494`
- Score split: 47 pass, 42 scored fail
- Soft timeouts: 13 total, 2 pass, 11 fail
- Provider/policy-like blocks: 5 trials
- Roder non-zero exec statuses: 6 trials

Earlier baseline run:

- Job: `evals/harbor/jobs/roder-tbench-full-gpt55-medium`
- Report: `evals/reports/harbor/roder-tbench-full-gpt55-medium-analysis.json`
- Model: `codex/gpt-5.5`
- Reasoning: `medium`
- Parallelism: 4 concurrent trials
- Result: 89 trials, 0 Harbor errors, mean `0.48314606741573035`
- Score split: 43 pass, 46 scored fail
- Soft timeouts: 21 total, 4 pass, 17 fail

Strict-run delta vs the earlier baseline:

- Net score movement: +4 passes.
- Improved tasks: `cancel-async-tasks`, `circuit-fibsqrt`, `code-from-image`,
  `financial-document-processor`, `llm-inference-batching-scheduler`,
  `mailman`, `reshard-c4-data`, `rstan-to-pystan`, `write-compressor`.
- Regressed tasks: `adaptive-rejection-sampler`, `break-filter-js-from-html`,
  `git-multibranch`, `password-recovery`, `vulnerable-secret`.

Target context from the benchmark leaderboard:

- Current GPT-5.5 state of the art: `NexAU-AHE`, 84.7% +/- 2.1,
  submitted 2026-05-14 by `china-qijizhifeng` with OpenAI.
- Acceptable near-term target: beat `Codex CLI`, 82.0% +/- 2.2, submitted
  2026-04-23 by OpenAI.
- Current Roder measurement is 56.2%, so score work needs broad failure
  reduction rather than one-off harness cleanup. Beating Codex CLI requires
  roughly 23 additional passes on this 89-task suite; matching current SOTA
  requires roughly 26 additional passes.

The strict run is Harbor-clean, but it exposed several reliability and
score-improvement problems that are worth addressing before treating this as a
stable benchmark loop.

## Recent Harness Changes

The smoke and full-run configs now set:

```json
"environment": {
  "delete": false
}
```

This disables Harbor post-run Docker cleanup so pulled Terminal-Bench task images
remain available for later offline preflight and targeted reruns.

Tradeoff: Docker disk usage will grow. Add a manual cleanup command or wrapper
option before running many full suites.

The full and smoke configs also set `speed_policy_enabled: false` in the Roder
agent kwargs. Without this explicit setting, `runtime_profile = "eval"` enables
Roder's speed policy defaults, which can move GPT-5.5 calls away from the
requested `medium` reasoning effort. Future `gpt-5.5 medium` Harbor runs should
therefore be strict-medium baselines unless the config opts back into speed
policy.

The smoke and full configs no longer override per-task Docker CPU, memory, or
storage limits. Harbor warns that those overrides can disqualify benchmark
submissions, so future score runs should use Terminal-Bench's task defaults
while keeping Harbor orchestrator parallelism at 4.

The `dna-assembly` provider/API bug has been fixed and rerun once:

- Source failure class: `provider_api_invalid_tool_name`, exit status 1.
- Fix: sanitize replayed function-call names and provider-metadata function-call
  names for the OpenAI Responses API while retaining local canonical names.
- Targeted rerun job: `evals/harbor/jobs/roder-tbench-provider-api-invalid-tool-name-fix`.
- Targeted rerun result: clean Harbor run, no provider/API hard failure, reward
  remained 0.0. This improved harness reliability but did not lift score.

The strict full run confirmed that the invalid Responses API tool-name failure
is gone: `dna-assembly` completed as a normal reward-0 scored failure instead of
a provider/API request error.

Deadline-aware exec and eval reliability config have now been implemented,
tested on a four-task failed subset, and run once on the full suite:

- Source failure classes: `soft_timeout_fail` and non-policy
  `roder_exec_error_status`.
- Config: `evals/reports/harbor/roder-tbench-deadline-exec-reliability-rerun.json`.
- Job: `evals/harbor/jobs/roder-tbench-deadline-exec-reliability-rerun`.
- Result: 4 trials, 0 Harbor errors, mean `0.5`, 2 pass, 2 fail.
- Flipped to pass: `mcmc-sampling-stan`, `qemu-startup`.
- Still failed: `compile-compcert`, `break-filter-js-from-html`.

Findings from that rerun:

- Raising `max_consecutive_tool_failures` from the eval default fixed
  `qemu-startup`; the strict run had stopped after five recoverable tool
  failures, while the rerun completed and scored 1.0.
- Deadline-aware `exec_command` now clamps long commands to the remaining eval
  deadline minus a finalization reserve. This makes command timeouts visible as
  tool results instead of only as external process interrupts.
- `compile-compcert` still exhausted the available eval window. Its final
  `make -j"$(nproc)"` command was clamped near the deadline and timed out with
  only about one second remaining, so this remains a task strategy or longer
  timeout problem rather than a missing timeout signal.
- `break-filter-js-from-html` moved from opaque soft timeout to a provider
  stream failure: `error decoding response body` after partial task work. That
  should be tracked separately from HTTP status retries because it occurs while
  consuming an accepted streaming response.

Full-run findings from
`roder-tbench-full-gpt55-medium-deadline-reliability`:

- The run was Harbor-clean: all 89 trials scored, with 0 setup/environment
  errors.
- Net score improved from 47/89 to 50/89, but this is still far below the
  82.0% Codex CLI target.
- Deadline-aware execution helped several timeout-heavy tasks become scoreable
  successes: `break-filter-js-from-html`, `extract-moves-from-video`,
  `mcmc-sampling-stan`, and `path-tracing-reverse`.
- Several wins were stochastic or planning-sensitive rather than purely harness
  driven: `adaptive-rejection-sampler`, `configure-git-webserver`,
  `git-multibranch`, `pypi-server`, and `pytorch-model-recovery`.
- Regressions show that one full run is not enough to declare task-level
  improvements stable: `financial-document-processor`,
  `fix-code-vulnerability`, `git-leak-recovery`, `kv-store-grpc`,
  `llm-inference-batching-scheduler`, and `query-optimize` all flipped from
  pass to fail.
- `fix-ocaml-gc` passed despite a soft timeout and repeated interrupted turns;
  the verifier then spent roughly 20 minutes compiling/testing OCaml before
  finalizing the run.
- `qemu-startup` passed in the four-task targeted rerun but failed in the full
  run, so the raised tool-failure tolerance is not sufficient by itself.
- Several failures were caused or worsened by thin task images missing common
  tools or packages (`curl`, `file`, `pkill`, image/OCR utilities, `pandas`).
- Provider policy blocking remains material: `crack-7z-hash`,
  `git-leak-recovery`, `model-extraction-relu-logits`, `password-recovery`,
  `sanitize-git-repo`, and `vulnerable-secret` matched policy-block signatures.

Targeted deadline-finalization smoke:

- Config:
  `evals/reports/harbor/roder-tbench-deadline-finalization-smoke-2.json`.
- Job: `evals/harbor/jobs/roder-tbench-deadline-finalization-smoke-2`.
- Task: `mteb-leaderboard`.
- Forced timing: 150-second internal eval deadline, 170-second adapter soft
  timeout, 180-second Harbor agent timeout.
- Result: 1 trial, 0 Harbor errors, reward `0.0`.
- Runtime signal: `roder-run-summary.json` recorded exit status 0, elapsed 142
  seconds, `soft_timed_out=false`, `deadline_timed_out=false`, and final
  `turn.completed`.
- Analyzer class: `deadline_finalized` plus `scored_fail`.
- Failure cause: Roder stopped gracefully before the deadline but did not write
  `/app/result.txt`; verifier expected `GritLM/GritLM-7B`.
- Harness finding: generated configs under `evals/reports/harbor` need
  `PYTHONPATH=$PWD/evals/harbor` when run directly because Harbor resolves
  custom agent imports relative to the config process path.

## Priority 0: Keep The Harness Reliable

### 1. Make Soft Timeouts Graceful Instead Of Abrupt

Evidence:
- Deadline/reliability run: 11 trials had soft timeouts.
- 8 of those 11 soft-timeout trials scored `0.0`.
- 3 soft-timeout trials scored `1.0`: `fix-ocaml-gc`,
  `mcmc-sampling-stan`, `path-tracing-reverse`.
- Strict run: 13 trials ended with `roder exec finished with status 124`.
- 11 of those 13 soft-timeout trials scored `0.0`.
- 2 soft-timeout trials still scored `1.0`: `fix-code-vulnerability`,
  `fix-ocaml-gc`.
- Earlier baseline: 21 soft timeouts, 17 failures, 4 passes.
- Every soft-timeout trial had an empty `roder-last-message.txt` and stderr `Error: interrupted`.

Problem:
The adapter prevents Harbor `AgentTimeoutError`, but it currently interrupts
`roder exec` without a finalization window. This is good for harness cleanliness
and bad for task completion, artifact quality, and score analysis.

Status:
- Implemented in the adapter and analyzer for new runs. Older runs still
  analyze through stderr/setup-summary text fallbacks.
- Harbor configs now collect `roder-run-summary.json` as a deterministic
  artifact.
- Implemented in Roder runtime. When the eval deadline reserve is reached,
  Roder injects a finalization prompt, disables tools for the next model
  request, skips verification-gate prompts, and completes with the best final
  answer available instead of waiting for a hard deadline failure.
- Harbor configs now set
  `speed_policy_eval_deadline_seconds: 870` and use an external soft timeout of
  890 seconds.
- `exec_command` receives remaining eval deadline context and clamps command
  timeouts to reserve a 30-second finalization window.
- The adapter classifies internal deadline expiry separately from Harbor hard
  timeout and keeps those trials scoreable.
- The analyzer splits `soft_timeout_pass`, `soft_timeout_fail`, and
  `internal_deadline_timeout`.

Remaining fix:
- Record the active command/tool when the timeout fired in the structured run
  summary. The first version records the last event but does not yet extract an
  active command/tool from Roder runtime state.
- Add repeated-run or targeted-rerun evidence before treating individual
  timeout flips as stable; the full run improved net score but also produced
  regressions.

Acceptance:
- Soft-timeout artifacts include a structured timeout reason, last active tool,
  elapsed time, and whether the agent attempted finalization.
- A forced-short runtime test completes normally, disables tools during
  finalization, and includes non-empty final answer text.
- Full-run soft timeouts are no longer opaque `Error: interrupted` events.

### 2. Treat Non-Zero Roder Exec Status As A First-Class Signal

Evidence:
- The deadline/reliability run had 9 `roder_exec_error_status` trials:
  `crack-7z-hash`, `git-leak-recovery`, `make-doom-for-mips`,
  `model-extraction-relu-logits`, `mteb-leaderboard`, `password-recovery`,
  `path-tracing-reverse`, `sanitize-git-repo`, `vulnerable-secret`.
- 6 trials matched `provider_policy_block`: `crack-7z-hash`,
  `git-leak-recovery`, `model-extraction-relu-logits`, `password-recovery`,
  `sanitize-git-repo`, `vulnerable-secret`.
- `path-tracing-reverse` and `sanitize-git-repo` scored `1.0` despite
  non-zero Roder status signatures, so score pass does not imply agent health.
- The strict run had 6 `roder_exec_error_status` trials:
  `crack-7z-hash`, `git-leak-recovery`, `model-extraction-relu-logits`,
  `password-recovery`, `qemu-startup`, `vulnerable-secret`.
- 5 of those 6 were also classified as `provider_policy_block`:
  `crack-7z-hash`, `git-leak-recovery`, `model-extraction-relu-logits`,
  `password-recovery`, `vulnerable-secret`.
- `git-leak-recovery` scored `1.0` despite a Roder exit status of `1`, which
  means the verifier can pass after the agent process exits non-zero.
- `qemu-startup` exited non-zero without matching the policy-block signature.
- The earlier `dna-assembly` invalid tool-name request failure has been fixed
  and did not recur in the strict run.
- The targeted deadline/reliability rerun exposed a new provider stream failure
  class in `break-filter-js-from-html`: `error decoding response body` after the
  turn had already emitted partial tool work.

Problem:
Reward tells us whether the task passed, but a non-zero Roder process status
tells us the agent did not finish normally. That should be visible as a harness
health and provider-policy signal even when the verifier still scores the task.

Fix:
- Keep analyzer classes for `provider_api_invalid_tool_name`,
  `provider_policy_block`, `provider_stream_decode_error`,
  `provider_stream_incomplete`, `roder_exec_error_status`, `soft_timeout_pass`,
  and `soft_timeout_fail`.
- Add a policy-block manifest with provider error code, final visible user task,
  and whether the verifier still passed.
- Add provider stream failure manifests with last emitted event id and whether
  the partial state was likely scoreable.
- Investigate whether eval/runtime policy mode should route benchmark security
  tasks through a trusted profile, a task-specific allowlist, or a clearer
  provider-limitation classification.
- Keep `qemu-startup` in the regression set, but the first cause is fixed by
  raising eval-mode consecutive tool failure tolerance.

Acceptance:
- Policy-blocked tasks are visible in a dedicated manifest rather than buried in
  `scored_fail`.
- A pass with non-zero Roder status is reported as "score pass, agent unhealthy"
  instead of silently treated as a clean pass.
- Analyzer `--require-clean` can optionally fail on provider/API hard failures,
  depending on whether the run target is "score measurement" or "harness health".

### 3. Improve Artifact Signal Quality

Evidence:
- All 89 trials produced deterministic artifacts.
- BEL-only stderr remains common in successful and failed trials.
- Soft-timeout stderr still commonly contains only `Error: interrupted`.
- Provider/API failures are now better classified, but the source signal is
  still mostly stderr and event text rather than a structured summary.
- `roder --version` was unavailable during setup self-test, so provenance records
  report the agent version as `unknown`.

Problem:
Artifacts exist, but some are too noisy or too thin for diagnosis. The analyzer
has to infer too much from stderr text.

Fix:
- Suppress or strip the BEL-only stderr artifact in eval mode.
- Add a structured `roder-run-summary.json` artifact with:
  - roder version/git SHA
  - model/provider/reasoning
  - exit status and signal
  - elapsed setup/agent/verifier time
  - soft-timeout flag
  - final active command/tool
  - provider error code if present
- Implement `roder --version` or `roder version` for setup provenance.

Status:
- First structured summary artifact is implemented for new runs with provider,
  model, reasoning, policy mode, config dir, soft timeout, eval deadline, start
  and finish times, elapsed seconds, exit status, soft/deadline timeout flags,
  deadline-finalized flag, provider error kind, artifact byte sizes, last event,
  and Roder version text when setup can collect it.
- Analyzer prefers summary fields for exit status, soft timeout, internal
  deadline timeout, deadline-finalized completion, and provider error
  classification.
- Remaining gaps are BEL-only stderr cleanup, active tool extraction, setup vs
  agent vs verifier time splits, and a real `roder version` command.

Acceptance:
- Analyzer can classify failures primarily from structured summary fields.
- BEL-only stderr no longer appears as diagnostic noise.
- Every trial records the exact Roder build used.

### 4. Preserve Image Cache Without Losing Disk Control

Evidence:
- With `environment.delete: true`, post-run offline preflight reported all 89
  task images missing because Harbor cleanup removed Docker state.
- The pre-run pull manifest proved all 89 images were present before launch.

Problem:
Post-run cleanup made immediate offline preflight and rerun workflows unreliable.
Disabling cleanup fixes that but can leave large Docker state behind.

Fix:
- Keep `environment.delete: false` for local reliability work.
- Add a documented cleanup command/script for explicit disk reclamation.
- Add an optional run wrapper guard that prints Docker disk usage before a full
  run and warns when projected usage is high.

Acceptance:
- Running a full suite no longer invalidates the local image cache.
- Cleanup is explicit and user-triggered.
- Offline preflight remains useful after a run unless the user manually prunes.

## Priority 1: Raise The Score

### 5. Build A Soft-Timeout Rerun Ladder

Evidence:
- The deadline/reliability run had 8 failed soft-timeout trials:
  `caffe-cifar-10`, `compile-compcert`,
  `llm-inference-batching-scheduler`, `make-doom-for-mips`,
  `make-mips-interpreter`, `mteb-leaderboard`, `train-fasttext`,
  `tune-mjcf`.
- The strict run had 11 failed soft-timeout trials:
  `break-filter-js-from-html`, `caffe-cifar-10`, `compile-compcert`,
  `extract-moves-from-video`, `install-windows-3.11`,
  `mcmc-sampling-stan`, `path-tracing-reverse`, `regex-chess`,
  `train-fasttext`, `tune-mjcf`, `winning-avg-corewars`.
- Several soft-timeout tasks are heavy compile, ML, QEMU, video, or symbolic
  search tasks.

Problem:
A single 840-second soft timeout makes the full run clean, but it does not tell
us whether score would improve with better deadline handling, larger task
timeouts, or smarter stopping.

Fix:
- Generate targeted configs for `soft_timeout_fail` using the deadline-aware
  harness settings as the new baseline.
- Rerun with a ladder such as 840s, 1200s, 1800s on a small subset.
- Compare reward, final message presence, event count, and active command at
  timeout.
- Use the result to decide between harness timeout policy changes and Roder
  planning/command-monitoring fixes.

Acceptance:
- We know which failed soft-timeout tasks are purely time-limited.
- The full-run timeout remains conservative, but targeted score work has a
  repeatable path.

### 6. Improve Long-Running Command Monitoring

Evidence:
- The top agent execution durations cluster at the 840-second soft timeout.
- Several successful non-soft tasks still ran for a long time, for example
  `query-optimize` ran about 828 seconds and passed.

Problem:
Roder may spend too long inside commands without enough task-level progress
assessment. Some tasks need long commands; others need earlier course correction.

Fix:
- Add eval-mode command heartbeat events with wall-clock duration and recent
  output snippets.
- Teach Roder to interrupt or background long commands when the remaining
  deadline is too short.
- Add task-local verifier discovery prompts: before spending many minutes,
  inspect tests/eval scripts and run fast probes.

Acceptance:
- Long commands emit enough metadata to distinguish useful work from hangs.
- More long tasks either complete before soft timeout or leave better partial
  state for scoring.

### 7. Add Domain-Focused Score Backlogs

Current scored-failure groups:
- ML/scientific: 10 tasks
- systems/emulation/services: 5 tasks
- media/geometry: 3 tasks
- synthesis/security/math: 5 tasks
- other: 16 tasks

Problems by group:
- ML/scientific tasks often need package strategy, numerical validation, and
  long-running experiment control.
- Systems/emulation/service tasks need background process handling, port checks,
  and service readiness validation.
- Media/geometry tasks need better binary/media artifact inspection and iterative
  verifier feedback.
- Synthesis/security/math tasks need exact-output discipline, search pruning,
  and strong local test loops.
- The `other` bucket is too broad and should be split after the first targeted
  rerun pass.

Fix:
- Keep score work out of the harness-cleanliness path.
- Create targeted rerun configs per group.
- For each group, inspect `roder-events.jsonl`, workspace diff, verifier output,
  and final artifact state before changing core Roder behavior.

Acceptance:
- Each group has a small measured rerun set.
- Changes are tied to observed task failures, not aggregate score alone.
- The analyzer groups shrink the `other` bucket into useful categories.

## Priority 2: Make Iteration Cheaper

### 8. Add Analyzer Deltas Between Runs

Status:
Implemented as `evals/harbor/compare_tbench_runs.py`.

Evidence:
- Generated
  `evals/reports/harbor/roder-tbench-full-gpt55-medium-strict-vs-deadline-reliability-comparison.json`.
- Generated
  `evals/reports/harbor/roder-tbench-full-gpt55-medium-strict-vs-deadline-reliability-comparison.md`.
- The comparison report shows +9 improved tasks, -6 regressed tasks, +3 net
  passes, and 8 class-only changes.

Acceptance:
- A rerun can produce a concise "changed tasks" report.
- Score experiments no longer require manually diffing two large JSON files.

### 9. Record Config Snapshots And Local Environment Details

Problem:
Per-trial `result.json` captures the Harbor config used for that trial, but the
top-level report does not summarize local variables such as binary path, source
mode, image preflight manifest, Docker cleanup policy, or wrapper environment.

Fix:
- Write a top-level run manifest next to the analysis report.
- Include:
  - config file checksum
  - prebuilt binary checksum
  - preflight manifest path and summary
  - `RODER_HARBOR_*` environment switches
  - Docker cleanup policy
  - Git commit/dirty status summary

Acceptance:
- A future run can be compared or reproduced without reconstructing shell state
  from chat logs.

## Suggested Next Implementation Order

1. Run a small `soft_timeout_fail` targeted rerun with deadline finalization
   enabled and compare it to the deadline/reliability full-run baseline.
2. Suppress BEL-only stderr and add active tool extraction to
   `roder-run-summary.json`.
3. Add provider stream decode/incomplete retry strategy only where replay is
   safe, and keep analyzer classification for unsafe mid-turn failures.
4. Add an eval image/tool preflight report for common task utilities and Python
   modules (`curl`, `file`, `pkill`, OCR/image tools, `pandas`) without mutating
   official task images for score submissions.
5. Run targeted reruns for `soft_timeout_fail`, provider stream failures, and
   policy-blocked tasks.
6. Split the scored-failure `other` bucket with artifact-backed categories.
7. Rerun unstable flips at parallelism 4 before counting them toward the
   82.0% target.
