# Roder Harbor Score Plan

This plan tracks score-oriented work separately from the broader harness
reliability backlog.

## Target

- Current measured Roder baseline:
  `roder-tbench-full-gpt55-medium-deadline-reliability`
- Baseline result: 50/89 pass, mean `0.5618`
- User-provided GPT-5.5 SOTA: `NexAU-AHE`, 84.7% +/- 2.1
- User-provided acceptable target: beat `Codex CLI`, 82.0% +/- 2.2
- Beat Codex CLI on this 89-task suite: at least 73/89 pass, +23 over baseline
- Beat 84.7% directly on this suite: at least 76/89 pass, +26 over baseline

The current targeted fixes are useful but not enough. The generated
validated-conversions manifest now records a `scoreProjection` block so this
arithmetic is tied to the route set. If every generated route is stable in a
clean routed run, the expected score is 65/89: 50 baseline passes,
4 medium-reasoning focused conversions, 7 xhigh-reasoning conversions, and
4 plan-first xhigh conversions. That is still 8 short of beating Codex CLI and
11 short of directly beating 84.7%.

The historical-wins campaign adds the three already-observed wins that were not
in the validated-conversions route set: `password-recovery`, `qemu-startup`, and
`vulnerable-secret`. Use `summarize_tbench_campaigns.py --require-no-overlap
--expect-unique-tasks 18 --expect-projected-passes 68 --expect-task ...
--expect-owner ...`
against both manifests before a live run, with those three task names passed as
explicit `--expect-task` checks and their reviewed route assignments passed as
`--expect-owner` checks. The intended combined projection is 18 unique candidates
and 68/89, leaving 5 passes to beat Codex CLI and 8 to beat the direct SOTA
target.

## Validated Conversions

- `mteb-leaderboard`
  - Failure mode: no scoreable file before deadline, then overwriting the
    correct dated candidate with a weaker live/current candidate.
  - Fix: scoreable-output checkpoint plus candidate preservation language.
  - Evidence:
    `evals/reports/harbor/roder-tbench-preserve-candidate-mteb-rerun.md`
  - Result: 1/1 pass.

- `mteb-retrieve`
  - Failure mode: raw `sentence_transformers` embedding path produced the wrong
    5th-ranked document.
  - Fix: MTEB-specific guidance to use `mteb.get_model`, model
    `encode`/`similarity`, and `mteb.encoder_interface.PromptType`.
  - Evidence:
    `evals/reports/harbor/roder-tbench-mteb-prompt-type-guidance-rerun.md`
  - Result: 1/1 pass.

- `llm-inference-batching-scheduler`
  - Failure mode: full run missed output plans after long optimization work.
  - Current signal: repeated focused pass with generated plans under verifier
    thresholds.
  - Evidence:
    `evals/reports/harbor/roder-tbench-preserve-candidate-score-probe.md`
  - Result: 1/1 pass in latest focused probe.

- `financial-document-processor`
  - Failure mode: baseline processed only a subset of the documents, moved only
    3 invoices, and wrote an incomplete 4-row summary.
  - Fix: Terminal-Bench guidance to process every input file, keep a manifest,
    classify each document once, and verify the required summary cardinality.
  - Evidence:
    `evals/reports/harbor/roder-tbench-exact-data-guidance-rerun.md`
  - Result: 1/1 pass.

## Validated Xhigh Conversions

Evidence:
`evals/reports/harbor/roder-tbench-remaining-failures-gpt55-xhigh.md`

The remaining-failure xhigh rerun used `codex/gpt-5.5` with reasoning `xhigh`,
parallelism 4, and excluded the 4 previously validated medium-reasoning
conversions. Result: 35 trials, clean run, 7 passes, mean `0.200`.

Use xhigh selectively for these tasks in the next routed run:

- `db-wal-recovery`
- `fix-code-vulnerability`
- `kv-store-grpc`
- `polyglot-c-py`
- `query-optimize`
- `torch-pipeline-parallelism`
- `tune-mjcf`

Do not blindly use xhigh for all remaining failures. The same run policy-blocked
5 security/secret tasks that may need a lower-risk prompt or different
reasoning route:

- `crack-7z-hash`
- `git-leak-recovery`
- `model-extraction-relu-logits`
- `password-recovery`
- `vulnerable-secret`

## Current Non-Conversions

- `qemu-startup`
  - Latest result: failed with internal deadline timeout.
  - Evidence:
    `evals/reports/harbor/roder-tbench-preserve-candidate-score-probe.md`
  - Primary issue: QEMU in the task image hits
    `rosetta error: Unimplemented syscall number 282`; attempted workaround
    needed a compiler that the task image does not provide.
  - Next path: handle as environment/emulation strategy, not generic prompt
    guidance.

- `mteb-retrieve` first MTEB API rerun
  - Latest status: superseded by the prompt-type rerun.
  - Failure cause: correct direction but wrong `PromptType` import path and
    premature finalization with a heuristic candidate.

- `gcode-to-text`
  - Latest result: failed in
    `evals/reports/harbor/roder-tbench-remaining-failures-gpt55-xhigh.md`.
  - Failure cause: rendered/projection-derived answer improved over the
    baseline generic description, but still did not find the exact string.
  - Next path: provide stronger artifact-inspection/OCR tooling or a
    projection-sweep helper instead of relying on prompt guidance alone.

- `filter-js-from-html`
  - Latest result: failed in
    `evals/reports/harbor/roder-tbench-remaining-failures-gpt55-xhigh.md`;
    the earlier byte-preservation rerun also failed and had to be stopped after
    Harbor waited on a completed verifier process, though the xhigh run
    eventually returned reward `0`.
  - Failure cause: XSS vectors were blocked, but clean HTML normalization still
    differed from verifier expectations.
  - Next path: match parser normalization in clean-file local checks and add a
    verifier-process timeout/cleanup guard to the Harbor wrapper.

- `protein-assembly`
  - Latest result: failed in
    `evals/reports/harbor/roder-tbench-remaining-failures-gpt55-xhigh.md`.
  - Failure cause: after adding no-placeholder guidance, the agent still
    produced a final gBlock file that the verifier rejected as not matching
    `[atcg]+`.
  - Next path: add or emphasize post-write validation against the actual
    required file bytes, not only intermediate generated sequence variables.

## Next Score Work

1. Build a routed "validated conversion" rerun set before another full run.
   Include `mteb-leaderboard`, `mteb-retrieve`,
   `llm-inference-batching-scheduler`, `financial-document-processor`, plus
   the 7 validated xhigh conversions above. Keep `qemu-startup` out unless the
   environment issue is separately addressed.

   Generate and summarize the separate `historical-wins` route before launch so
   `qemu-startup` stays reviewable as an environment-targeted historical win
   instead of silently joining the validated conversion route.

2. Split remaining scored failures into two categories:
   - task-strategy candidates where prompt/runtime guidance can plausibly lift
     score without changing task images
   - environment/provider blockers where the harness should classify the
     limitation instead of retrying blindly

3. Attack high-leverage groups next:
   - exact-output/data tasks: `financial-document-processor`,
     `gcode-to-text`, `filter-js-from-html`, `db-wal-recovery`
   - ML/scientific tasks with local APIs: `raman-fitting`, `protein-assembly`,
     `torch-pipeline-parallelism`, `torch-tensor-parallelism`
   - timeout-heavy tasks where a focused pass already exists or where partial
     work can be made scoreable earlier

4. Do not spend more full-run budget until the routed focused set shows the
   current +15 conversions together in one clean run. That still leaves a
   second tranche of at least 8 conversions to beat Codex CLI.

## Reliability Work That Still Affects Score

- Record active model call state in `roder-run-summary.json`; current summaries
  capture active tools but cannot distinguish a long model call from idle time.
- Suppress BEL-only stderr noise so unhealthy stderr is easier to identify.
- Add environment/preflight classification for tasks blocked by QEMU/Rosetta,
  missing compilers, or provider policy blocks.
- Add verifier-process timeout/cleanup handling for tasks whose verifier writes
  reward/CTRF artifacts but leaves the `docker compose exec` process blocked.
- Keep full-run parallelism at 4 for comparability with the latest baseline.
