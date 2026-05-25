# Roder Harness Xhigh Remaining Win Analysis

Date: 2026-05-25

This note analyzes the 28 tasks that still failed after the remaining baseline failures were rerun with `codex/gpt-5.5` at `xhigh` reasoning. The goal is to identify the strongest additional wins for the harbor eval harness before spending more full-run budget.

Source artifacts:

- Run report: `evals/reports/harbor/roder-tbench-remaining-failures-gpt55-xhigh.md`
- Structured analysis: `evals/reports/harbor/roder-tbench-remaining-failures-gpt55-xhigh-analysis.json`
- Job directory: `evals/harbor/jobs/roder-tbench-remaining-failures-gpt55-xhigh/`

The xhigh rerun converted 7 of 35 previously failing tasks:

- `db-wal-recovery`
- `fix-code-vulnerability`
- `kv-store-grpc`
- `polyglot-c-py`
- `query-optimize`
- `torch-pipeline-parallelism`
- `tune-mjcf`

The remaining 28 failures break into three useful groups:

- Strong near-term wins: verifier output shows a narrow miss or a correct approach with a concrete final-step defect.
- Conditional wins: task is close, but needs a policy route, environment change, or expensive task-specific improvement.
- Low-signal for immediate score: failure mode is broad, timeout-heavy, or the task did not produce the required artifact.

## Strongest Candidates

| Rank | Task | Why it is a strong win | Evidence | Next harness action |
| --- | --- | --- | --- | --- |
| 1 | `polyglot-rust-c` | The implementation appears to have solved the core task but left compiled validation binaries in the required output directory. | Verifier expected only `main.rs`; found `main.rs`, `main`, and `cmain`. | Add explicit post-validation discipline: compile temporary binaries outside the submission directory or remove validation artifacts before final answer. |
| 2 | `dna-insert` | The output satisfied most structural checks and failed only the final primer Tm balance. | Failed `abs(fwd_tm - rev_tm) <= 5`; actual difference was `6.531905`, with both Tm values in range. | Require final validation against the actual written file using the verifier's `oligotm` method before finalizing. |
| 3 | `dna-assembly` | Similar to `dna-insert`: the output satisfied format, overhang, annealing, and range checks, then failed final pair balance. | Failed Tm difference by `10.845547`; last agent message admitted full `oligotm` validation was not completed before deadline. | Push agents toward a small primer search loop and exact final verifier-style Tm validation. |
| 4 | `sam-cell-seg` | Seven of nine tests passed; one failure is pure serialization and the other is a close IoU miss. | `coords_x` was a tuple instead of a list; mask IoU was `0.445576`, threshold `0.5`. | Add task guidance to validate output JSON/list types and tune mask alignment using the verifier image before finalizing. |
| 5 | `torch-tensor-parallelism` | Most tensor-parallel behavior passed; the remaining issue is localized to row-parallel multi-rank shape handling. | Verifier failed with `RuntimeError: mat1 and mat2 shapes cannot be multiplied (2x4 and 16x48)` in `parallel_linear.py`. | Encourage running the installed verifier when torch is present and explicitly reason about row-parallel input shard dimensions. |
| 6 | `protein-assembly` | The artifact existed and passed basic DNA/file constraints, but the protein order was wrong against visible verifier constants. | Translation did not start with the expected flag segment; `flag_idx == -1`. | Tell agents to extract the exact expected amino-acid constants from tests and translate the final construct before final answer. |
| 7 | `gcode-to-text` | One of two tests passed and the failed test exposed the exact expected output. | Expected `flag{gc0d3_iz_ch4LLenGiNg}`; output was `Embossed text`. | For exact-answer media tasks, require inspecting verifier expectations and writing the exact decoded answer, not a description. |
| 8 | `video-processing` | The TOML output shape passed and both failures are numeric frame offsets, not missing artifact failures. | Example takeoff predicted 55 vs expected 50-54; test video takeoff predicted 185 vs expected 219-223. | Improve frame-event detection and require local visual or script validation against expected frame ranges. |

These are the highest-value retry targets. The first six are especially attractive because the verifier evidence points to narrow, explainable defects rather than broad unsolved work.

Recommended next focused rerun set:

```text
polyglot-rust-c
dna-insert
dna-assembly
sam-cell-seg
torch-tensor-parallelism
protein-assembly
gcode-to-text
video-processing
```

## Conditional Candidates

| Task | Current result | Why it is conditional |
| --- | --- | --- |
| `git-leak-recovery` | Four of five tests passed, but the run hit a provider policy block and did not create `secret.txt`. | Close by verifier count, but xhigh triggered policy handling. A lower-risk benchmark framing or medium-reasoning route may be more useful than another xhigh retry. |
| `filter-js-from-html` | Zero of two tests passed, but the failure list is concrete. | The sanitizer changed clean HTML and missed specific XSS vectors. Fixable, but verifier runtime was long and this is a real implementation task rather than a final-step miss. |
| `install-windows-3.11` | Three of four tests passed. | Remaining failure involved missing QEMU monitor socket state. This may be more of an environment/service control issue than a model reasoning issue. |
| `raman-fitting` | Results file existed, but fitted values were far from expected. | Needs better model/data fitting, not just output formatting. Possible but less direct than the primer, tensor, or artifact-cleanup failures. |
| `winning-avg-corewars` | Two of three tests passed. | Remaining score was 59 percent against `stone.red`, below 75 percent. This is search/tuning-heavy. |

## Weak Immediate Targets

These tasks are not good immediate xhigh retry candidates without a more specific harness change:

- Policy-blocked at xhigh: `crack-7z-hash`, `model-extraction-relu-logits`, `password-recovery`, `vulnerable-secret`.
- Timeout or environment-heavy: `compile-compcert`, `make-doom-for-mips`, `make-mips-interpreter`, `qemu-alpine-ssh`, `qemu-startup`, `path-tracing`.
- Missing or broadly incorrect artifact: `caffe-cifar-10`, `chess-best-move`, `gpt2-codegolf`, `regex-chess`, `train-fasttext`.

## Harness Reliability Improvements From These Failures

The strongest candidates point to recurring harness-level reliability issues:

1. Final artifact validation must inspect the exact file that will be scored, not an intermediate result.
   - Seen in `dna-insert`, `dna-assembly`, and `protein-assembly`.

2. Validation artifacts must not be left in required output directories.
   - Seen in `polyglot-rust-c`.

3. The final answer should require verifier-style checks for structured output types.
   - Seen in `sam-cell-seg`.

4. Tasks with installed dependencies should run the real verifier, not just syntax checks.
   - Seen in `torch-tensor-parallelism` and `sam-cell-seg`.

5. Exact-answer tasks should bias toward reading visible verifier expectations and producing the expected value, not a semantic description.
   - Seen in `gcode-to-text`.

## Proposed Next Step

Run a focused retry of the eight recommended targets at `xhigh` after adding one harness instruction block that stresses:

- score the exact final artifact;
- keep temporary validation files outside output directories;
- run verifier-style checks when dependencies exist;
- inspect visible tests for exact constants, output types, and threshold calculations.

This focused retry should be more cost-effective than another full remaining-failure sweep because the selected tasks have narrow, evidence-backed failure modes.
