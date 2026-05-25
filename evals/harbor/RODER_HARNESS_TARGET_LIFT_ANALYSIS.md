# Roder Harness Target Lift Analysis

Date: 2026-05-25

## Current Score Picture

The clean selected campaign result is 61/89, or 68.5%.

- Baseline full GPT-5.5 medium run: 50/89.
- GPT-5.5 xhigh rerun over medium failures: +7.
- Plan-first xhigh rerun over remaining failures: +4.

To beat the Codex CLI GPT-5.5 score of 82.0% on this 89-task set, Roder needs
73/89, so the clean campaign needs 12 more passes.

To beat the reported GPT-5.5 SOTA score of 84.7%, Roder needs 76/89, so the
clean campaign needs 15 more passes.

There is also a best-of historical signal across all targeted artifacts on
disk: 68/89, or 76.4%. That is not a clean campaign score, but it shows that
seven additional tasks have already passed under some harness mode and should
be folded into the normal run before chasing harder fixes:

- `financial-document-processor`
- `llm-inference-batching-scheduler`
- `mteb-leaderboard`
- `mteb-retrieve`
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
   61/89 toward 68/89.

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

1. Historical-wins consolidation batch.

   Target the seven tasks that have already passed in historical artifacts and
   encode the winning guidance into the normal harness route. The goal is to
   make 68/89 reproducible without oracle aggregation.

2. Verifier-contract batch.

   Target `sam-cell-seg`, `video-processing`, `torch-tensor-parallelism`,
   `dna-insert`, `dna-assembly`, `protein-assembly`, and `gcode-to-text`.
   The goal is to prove the final-check loop converts near misses.

3. Environment-target batch.

   Target `qemu-alpine-ssh`, `install-windows-3.11`, `qemu-startup`, and
   `train-fasttext`. The goal is to prove the agent is validating against the
   same endpoint and dependency surface as the verifier.

If the historical wins are made clean and the verifier-contract batch converts
even half of the near misses, Roder should be in range of the Codex CLI target.
The SOTA target likely needs those plus either the policy-shaped tasks or one
of the heavier solver classes.
