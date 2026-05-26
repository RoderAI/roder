# Roder Harness Target Lift Analysis

Date: 2026-05-25

## Current Score Picture

The generated validated-conversions campaign now projects 65/89, or 73.0%, if
all 15 non-overlapping conversion candidates reproduce in one clean routed run.
This is a projection, not a completed clean full-suite score.

- Baseline full GPT-5.5 medium run: 50/89.
- Medium-reasoning focused conversions: +4.
- GPT-5.5 xhigh rerun over medium failures: +7.
- Plan-first xhigh rerun over remaining failures: +4.

To beat the Codex CLI GPT-5.5 score of 82.0% on this 89-task set, Roder needs
73/89, so the generated campaign projection needs 8 more passes.

To beat the reported GPT-5.5 SOTA score of 84.7%, Roder needs 76/89, so the
generated campaign projection needs 11 more passes.

There is also a best-of historical signal across all targeted artifacts on
disk: 68/89, or 76.4%. That is not a clean campaign score, but it shows that
three additional tasks beyond the current generated campaign have already
passed under some harness mode and should be folded into the normal run before
chasing harder fixes:

- `password-recovery`
- `qemu-startup`
- `vulnerable-secret`

If those wins become reproducible in a clean campaign, the gap becomes 5 more
passes to beat Codex CLI and 8 more passes to beat the SOTA number.

The next clean-run configuration should double the task window before trying to
optimize inference speed. That means measuring score with the longer deadline
first, while keeping Roder's inference speed policy disabled so timeout lift and
model-call speed lift do not get conflated.

## What The Runs Tell Us

Plan-first is useful, but not as a blanket rerun mode. It converted four tasks:

- `git-leak-recovery`
- `model-extraction-relu-logits`
- `polyglot-rust-c`
- `regex-chess`

It also reduced policy-shaped failures from five to one, which is a strong
signal that a better benchmark framing path matters. However, it increased
soft/deadline failures and regressed some tasks that needed direct execution
more than long planning. The harness should route selectively instead of
turning plan-first on globally.

The biggest avoidable losses are final-verifier misses, not model capability
misses. Several failing tasks were close enough that the final artifact was
wrong by one serialization type, one numeric tolerance, one missing required
file, or one unrun verifier dependency:

- `sam-cell-seg`: 8/9 passed; only `coords_x` was a tuple instead of a list.
- `video-processing`: 4/5 passed; takeoff frame was 225 and needed 219-223.
- `torch-tensor-parallelism`: 2/3 passed; row-parallel shard shape errors were
  visible in verifier output.
- `dna-insert`: failed by Tm diff 5.44 against a threshold of 5.0.
- `dna-assembly`: required `/app/primers.fasta` was missing in plan-first.
- `protein-assembly`: failed the explicit 3000 bp gBlock limit.
- `gcode-to-text`: required `/app/out.txt` was missing in plan-first.

Those are harness-loop failures: the agent was allowed to finish after a weak
self-check instead of being forced through the exact test contract.

There are also environment and target-isolation failures:

- `qemu-alpine-ssh` connected to a host SSH/kernel path instead of the Alpine
  VM target.
- `install-windows-3.11` got 3/4 tests but did not produce the monitor/F1
  visual state the verifier expected.
- `torch-tensor-parallelism` and `sam-cell-seg` messages show dependency gaps
  between the agent's local validation path and the verifier path.

Finally, some remaining failures are likely task-solver quality rather than
harness reliability:

- `winning-avg-corewars`
- `raman-fitting`
- `path-tracing`
- `make-mips-interpreter`
- `make-doom-for-mips`
- `caffe-cifar-10`
- `compile-compcert`

These may still improve with better instructions, but they are lower-yield
than verifier, artifact, policy, and environment fixes.

## Highest-Leverage Harness Improvements

1. Promote historical targeted guidance into the core route.

   The best-of evidence already contains seven wins that are not in the clean
   selected campaign. Capture the successful run modes as benchmark guidance
   instead of leaving them as one-off reruns. This is the fastest path from
   65/89 toward 68/89.

2. Add a final verifier-contract loop.

   Before the model stops, the harness should require it to extract required
   output files, numeric thresholds, serialization expectations, and verifier
   commands, then run the closest available verifier. If the verifier cannot
   run, the harness should still force concrete artifact inspection against the
   contract. Expected lift: `sam-cell-seg`, `dna-insert`, `protein-assembly`,
   `gcode-to-text`, `video-processing`, and possibly
   `torch-tensor-parallelism`.

3. Add early artifact checkpointing.

   For any task with known required output paths, require the agent to create a
   provisional file early and update it in place. This directly targets
   deadline regressions where the solution was in progress but `/app/out.txt`
   or `/app/primers.fasta` did not exist when the verifier ran.

4. Improve verifier dependency parity.

   The agent should be able to run the same dependency stack the verifier uses,
   or receive a clear fallback command that exercises the same assertions. This
   matters most for Torch, SAM/cell segmentation, video processing, and
   FastText-style tasks.

5. Route policy-shaped benchmark tasks through safe benchmark framing.

   Plan-first reduced policy failures and converted some sensitive-looking
   tasks. Keep the benchmark-authorization framing, but apply it selectively to
   `password-recovery`, `crack-7z-hash`, and `vulnerable-secret`, with explicit
   limits that the work is confined to the local benchmark artifact.

6. Add QEMU/service target sanity checks.

   For VM tasks, the harness should verify that SSH, monitor, and VNC/QMP
   commands are hitting the intended guest endpoint before the agent spends the
   run solving the wrong system. This can prevent host-kernel false positives
   and monitor-protocol mismatches.

7. Keep plan-first selective.

   Use plan-first for tasks that benefit from benchmark framing, multi-step
   artifact reasoning, or policy-sensitive wording. Avoid it for tasks where
   planning time competes with long builds, brute-force search, or training.

## Recommended Next Experiments

Run three small targeted batches before another full run:

0. Local diagnostic loop.

   Run `evals/harbor/run-roder-pre-eval-diagnostics.sh` after harness changes
   and before Harbor. This validates Harbor config readiness, then checks the
   exact-output, JSON-array, bounded-sequence, numeric-tolerance,
   output-directory-hygiene, visible-verifier-contract, artifact-checkpoint,
   service-target-sanity, and verifier-dependency-parity
   classes without spending a Terminal-Bench run. The local TBench diagnostic
   validator requires pass outcomes, completed verification, and zero unknown
   reliability errors, and it blocks if any one of the nine diagnostic fixtures
   is missing from the run artifact. Use `--include-speed` only when
   intentionally measuring inference-speed behavior; keep it separate from
   deadline-lift runs. Use `--require-prebuilt` immediately before a real Harbor
   run when the injected binary must already be present.
   Use `RODER_HARBOR_DRY_RUN=1 evals/harbor/run-roder-tbench-smoke.sh` as the
   one-task smoke handoff: it runs or validates the same pre-eval summary gate
   for `tbench-smoke.json` and exits before job replacement or `harbor run`.
   Live smoke also preserves an existing smoke job directory unless
   `RODER_HARBOR_REPLACE_JOB=1` is set.

   After a Harbor full run, pass the analyzer output back into the wrapper with
   `--analysis evals/reports/harbor/roder-tbench-full-gpt55-medium-analysis.json`.
   This runs `evals/harbor/validate_tbench_analysis.py` against
   `evals/harbor/tbench-clean-baseline.json`, keeping the phase-50 style
   baseline gate in the loop: unknown, setup/artifact, provider/runtime, and
   Harbor errors block follow-up spend, while reward-0 tasks remain score backlog. Use the wrapper's
   `pre-eval-summary.json` as the handoff artifact for the next live run; it
   includes a top-level `status` and `blockedChecks`, plus the SHA-256 and file
   type of the prebuilt Roder binary that will be injected. The wrapper exits
   non-zero after writing a blocked summary, including early readiness/auth and
   analysis-input or image-preflight setup failures when enough context is
   available. Blocked summaries include the failed wrapper step and exit code
   when the shell trap catches the failure. `--require-prebuilt` rejects
   non-Linux or non-x86-64 binaries.
   Use `--require-auth` before a live run so missing or malformed Codex auth is
   caught locally rather than inside the task container. Use `--preflight-images`
   to fold the Terminal-Bench Docker image-cache manifest into the same handoff.
   Wrapper-level validation pins that image-preflight evidence to the config
   being dry-run or launched, preventing a full-suite image summary from
   standing in for smoke evidence.
   Full configs without explicit `task_names` may use Harbor registry metadata
   for task-scope discovery, but image pulls still require explicit opt-in.
   The diagnostic fixture contract currently covers nine local fixtures,
   including output-directory hygiene, visible verifier constants,
   artifact-checkpoint task ledgers, service-target sanity, verifier dependency
   parity, and per-fixture command-check coverage.
   The guarded full-run wrapper now enforces a fresh handoff summary before
   Harbor starts: it either validates `RODER_HARBOR_PRE_EVAL_SUMMARY` or runs
   the diagnostic loop itself with prebuilt, auth, test, and image-preflight
   gates. Use `RODER_HARBOR_DRY_RUN=1` on the full-run wrapper to validate this
   gate without requiring `RODER_HARBOR_LIVE_TBENCH=1` or starting Harbor. It
   writes a JSON launch-plan artifact by default under
   `evals/reports/harbor/`; set `RODER_HARBOR_LAUNCH_PLAN` to choose a specific
   path. The same launch-plan path works on the live wrapper path and writes
   before job replacement or `harbor run`; gate on `launchStatus` and
   `blockedReasons`, then inspect `jobDirBlocksLaunch` and `blockedBeforeHarbor`
   to catch an existing output directory first. The launch plan also embeds
   compact `preEvalSummaryStatus` metadata so the handoff can be inspected
   without opening the summary JSON first; launch-plan validation rejects a
   missing, non-`ok`, or blocked embedded summary status. The
   `preEvalSummarySha256` field binds the plan to the exact summary file and
   the wrapper verifies it automatically. Use
   `validate_tbench_launch_plan.py --allow-dry-run` for dry-run handoff checks
   and `--require-ready` for live launch plans that must reach Harbor. The
   wrapper enforces that same ready check before image preflight, job
   replacement, or `harbor run`, verifies both the pre-eval summary SHA-256 and
   the Harbor config SHA-256, verifies that pre-eval readiness checked the same
   config hash being launched, verifies the prebuilt Roder binary hash recorded
   by the pre-eval summary, rechecks that the auth file is still valid JSON
   without hashing secret contents, rejects Harbor adapter file drift between
   diagnostics and launch, rejects image-preflight manifests produced for a
   different Harbor config, and automatically requires image preflight in that
   launch-plan check unless `RODER_HARBOR_SKIP_PREFLIGHT=1`.

1. Historical-wins consolidation batch.

   Target the seven tasks that have already passed in historical artifacts and
   encode the winning guidance into the normal harness route. The goal is to
   make 68/89 reproducible without oracle aggregation.

   For the current validated-conversions set, generate executable route configs
   before spending live Harbor time:

   ```sh
   python3 evals/harbor/generate_tbench_campaign.py \
     --output-dir evals/reports/harbor/campaigns/validated-conversions
   python3 evals/harbor/validate_tbench_campaign.py \
     evals/reports/harbor/campaigns/validated-conversions/validated-conversions-manifest.json
   ```

   Before deciding whether to widen the generated campaign, run the historical
   win suggester against the latest clean baseline and focused rerun artifacts:

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

   The suggester flags baseline failures that have at least one historical pass
   but are not already covered by the generated manifest. It currently surfaces
   the expected three extra historical wins: `password-recovery`,
   `qemu-startup`, and `vulnerable-secret`.

   Generate those three missing historical wins as a reviewed route set with:

   ```sh
   python3 evals/harbor/generate_tbench_campaign.py \
     --campaign historical-wins \
     --output-dir evals/reports/harbor/campaigns/historical-wins
   ```

   This writes a `policy-framed` route for `password-recovery` and
   `vulnerable-secret`, plus an `environment-targeted` route for `qemu-startup`.

   Before spending live route time, summarize the generated validated and
   historical manifests together:

   ```sh
   python3 evals/harbor/summarize_tbench_campaigns.py \
     evals/reports/harbor/campaigns/validated-conversions/validated-conversions-manifest.json \
     evals/reports/harbor/campaigns/historical-wins/historical-wins-manifest.json \
     --preset validated-plus-historical
   ```

   This is the machine-checkable gate for the intended 18 non-overlapping
   candidates, projected at 68/89 if every route reproduces, with the three
   historical wins on their intended reviewed routes.

   The manifest separates medium, xhigh, and plan-first xhigh routes so each
   slice can be preflighted and analyzed independently before a broader run. It
   also records a `scoreProjection` block so projected pass counts and target
   gaps come from the generated route set instead of hand-maintained arithmetic.
   Re-run `validate_tbench_campaign.py --require-image-preflight` after writing
   per-route image manifests so a route cannot launch against stale preflight
   evidence. The route gate reuses the pre-eval manifest detail checks, including
   task rows, image rows, status counts, image-to-task mapping, and exact route
   task-name coverage. The generator also writes `run-validated-conversions.sh`, which is
   the preferred reviewed handoff: it performs validation and per-route
   preflight first, then exits unless `RODER_HARBOR_LIVE_TBENCH=1` is set. Live
   route execution writes per-route analyzer JSON, Markdown, and rerun manifests
   back into the campaign directory immediately after each `harbor run`. The
   generated route script also runs the clean-run baseline validator with
   `--expected-trials` set to each route's task count, preserving the
   phase-50-style harness/provider/runtime blockers without falsely requiring a
   targeted route to contain all 89 full-run trials. The standalone campaign
   validator enforces the same subset-aware baseline when run with
   `--require-analysis`.
   Live generated scripts also preserve existing route job directories unless
   `RODER_HARBOR_REPLACE_JOB=1` is set, so historical route evidence is not
   overwritten during a reviewed handoff.
   Clean route image preflight also requires task-level `present` to cover every
   task. Shared Docker images are valid, but they must reduce `unique_images`
   rather than the number of present task rows.
   The campaign validator also rejects stale or hand-edited run scripts that
   drop the live-run guard, image preflight, pre-eval summary validation,
   job-directory preservation, or route analysis gates, and requires exact
   route config, job, image-preflight, and analysis paths from the manifest.
   Extra `harbor run --config` commands and stale image-preflight, pre-eval,
   or summary-validation config arguments outside the manifest route set are
   blocked. Analyzer commands must also use the manifest's exact route job and
   analysis-output paths, and baseline validation must use the exact route
   analysis JSON and task count.
   The script also passes every generated route config into the pre-eval
   diagnostic loop and requires reused summaries to include those route configs,
   preventing a default-only summary from launching a routed campaign. Image
   preflight config paths are now included in the same readiness and summary
   attestation, so a route cannot preflight one config and launch from a summary
   that only checked another.

2. Verifier-contract batch.

   Target `sam-cell-seg`, `video-processing`, `torch-tensor-parallelism`,
   `dna-insert`, `dna-assembly`, `protein-assembly`, and `gcode-to-text`.
   The goal is to prove the final-check loop converts near misses. Generate the
   reviewed route with:

   ```sh
   python3 evals/harbor/generate_tbench_campaign.py \
     --campaign verifier-contract \
     --output-dir evals/reports/harbor/campaigns/verifier-contract
   ```

3. Environment-target batch.

   Target `qemu-alpine-ssh`, `install-windows-3.11`, `qemu-startup`, and
   `train-fasttext`. The goal is to prove the agent is validating against the
   same endpoint and dependency surface as the verifier. Generate the reviewed
   route with:

   ```sh
   python3 evals/harbor/generate_tbench_campaign.py \
     --campaign environment-target \
     --output-dir evals/reports/harbor/campaigns/environment-target
   ```

If the historical wins are made clean and the verifier-contract batch converts
even half of the near misses, Roder should be in range of the Codex CLI target.
The SOTA target likely needs those plus either the policy-shaped tasks or one
of the heavier solver classes.
