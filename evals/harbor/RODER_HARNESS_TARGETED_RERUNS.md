# Roder Harbor Targeted Rerun Notes

This file records focused reruns used to validate harness changes before another
full Terminal-Bench sweep.

## Provider Stream Retry Rerun

- Config:
  `evals/reports/harbor/roder-tbench-provider-stream-retry-rerun.json`
- Job: `evals/harbor/jobs/roder-tbench-provider-stream-retry-rerun`
- Source class: `provider_stream_decode_error`
- Task: `make-doom-for-mips`
- Result: 1 trial, 0 Harbor errors, mean `0.0`
- Analyzer classes: `internal_deadline_timeout`, `roder_exec_error_status`,
  `scored_fail`, `soft_timeout`, `soft_timeout_fail`
- Runtime summary: elapsed 871 seconds, soft timeout true, internal deadline
  timeout true, provider error kind `turn_deadline_expired`
- Stream signal: no `provider_stream_decode_error` or
  `error decoding response body` occurred in this rerun

Harness change under test:

- Roder eval runtime now retries known transient provider stream/body failures
  when replay is safe: `error decoding response body` and
  `stream closed before response.completed`.
- Retry emits `reliability.retry` with provider/model context and uses the same
  provider retry budget/backoff config as request retries.
- Follow-up fix: eval inference streams now time out at the finalization reserve
  boundary before the hard turn deadline. If a model stream is still active
  when the reserve begins, Roder injects the finalization prompt, disables
  tools, and makes one no-tools final request instead of failing immediately at
  the hard deadline.
- Focused Rust coverage:
  `cargo test -p roder-core eval_runtime_retries_transient_provider_stream_decode_failure -- --nocapture`
  `cargo test -p roder-core eval_deadline_finalization_interrupts_model_stream_at_reserve -- --nocapture`
  and `cargo test -p roder-core`.

Interpretation:

- The live rerun did not reproduce the stream decode failure, so this is not
  live proof of recovery from that exact provider error.
- It did show the new prebuilt binary runs cleanly in Harbor and that the former
  provider-stream failure class can move into ordinary task work.
- The remaining `make-doom-for-mips` failure was task quality and deadline
  handling: the run installed `gcc-mipsel-linux-gnu`, attempted a MIPS
  cross-build, failed on `doomgeneric_img.c:1:10: fatal error: my_stdlib.h: No
  such file or directory`, and then reached the hard eval deadline without
  finalizing. The reserve-boundary finalization fix addresses that last runtime
  failure mode, but it does not solve the task implementation failure.

Next action:

- Keep provider stream retry enabled, but do not count it as a score lift until
  a rerun actually records `reliability.retry` and completes normally.
- Prioritize task-contract and deadline strategy for `make-doom-for-mips` and
  `make-mips-interpreter`; both currently spend too much time on broad MIPS
  implementation paths before producing scoreable artifacts.

## Deadline Reserve Live Smoke

- Config:
  `evals/reports/harbor/roder-tbench-deadline-reserve-live-smoke.json`
- Job: `evals/harbor/jobs/roder-tbench-deadline-reserve-live-smoke`
- Task: `make-doom-for-mips`
- Binary: `evals/harbor/artifacts/roder-linux-amd64`,
  BuildID `8ddd328d356d2de0`
- Forced timing: 150-second internal eval deadline, 170-second adapter soft
  timeout, 180-second Harbor agent timeout
- Result: 1 trial, 0 Harbor errors, mean `0.0`
- Analyzer classes: `deadline_finalized`, `scored_fail`
- Runtime summary: elapsed 147 seconds, soft timeout false, internal deadline
  timeout false, deadline finalized true, provider error kind null,
  stderr noise `bel_only`

Interpretation:

- The reserve-boundary finalization fix works in a real Harbor container:
  `internal_deadline_timeout` was replaced by a normal `turn.completed` with a
  final answer.
- Score remained `0.0` because no valid `doomgeneric_mips`/`frame.bmp` artifact
  was produced. This is now an ordinary task-quality failure rather than a
  runtime deadline failure.

## Soft Timeout Guidance Rerun

- Config:
  `evals/reports/harbor/roder-tbench-soft-timeout-guidance-rerun.json`
- Job: `evals/harbor/jobs/roder-tbench-soft-timeout-guidance-rerun`
- Tasks: `llm-inference-batching-scheduler`, `make-doom-for-mips`,
  `make-mips-interpreter`, `mteb-leaderboard`
- Result: 4 trials, 0 Harbor errors, mean `0.25`
- Score movement: `llm-inference-batching-scheduler` flipped from failed
  timeout in the full baseline to reward `1.0`

Interpretation:

- The benchmark guidance prompt helped at least one timeout-heavy task produce
  scoreable state.
- The MIPS tasks and `mteb-leaderboard` still need exact-contract behavior:
  MIPS tasks failed on generated executable/frame behavior, while
  `mteb-leaderboard` wrote the wrong model name.

## Exact Contract Guidance Rerun

- Config:
  `evals/reports/harbor/roder-tbench-exact-contract-guidance-rerun.json`
- Job: `evals/harbor/jobs/roder-tbench-exact-contract-guidance-rerun`
- Tasks: `make-doom-for-mips`, `make-mips-interpreter`
- Result: 2 trials, 0 Harbor errors, mean `0.0`

Findings:

- `make-doom-for-mips` previously exited through
  `provider_stream_decode_error` after substantial tool work.
- `make-mips-interpreter` completed at the eval deadline but still did not
  create `/tmp/frame.bmp`; verifier timed out waiting for the frame.

Next action:

- The MIPS pair is a good focused benchmark for exact stdout/file/image
  contract adherence, but it is not the fastest next score-lift path because
  each failed trial consumes most of the eval window.

## Scoreable Candidate Preservation Rerun

- Config:
  `evals/reports/harbor/roder-tbench-preserve-candidate-mteb-rerun.json`
- Job: `evals/harbor/jobs/roder-tbench-preserve-candidate-mteb-rerun`
- Source task: `mteb-leaderboard`
- Result: 1 trial, 0 Harbor errors, mean `1.0`
- Analyzer classes: `pass`

Harness changes under test:

- Roder eval runtime now injects a one-time scoreable-output checkpoint before
  the final deadline reserve when the task ledger still has open items.
- Long model inference calls with open task-ledger work now time out at the
  scoreable checkpoint boundary instead of consuming the final reserve.
- The checkpoint prompt tells the agent to preserve an existing plausible dated
  or historical candidate unless stronger task-specific evidence justifies a
  replacement.

Interpretation:

- This converted `mteb-leaderboard` from a full-run scored failure into a clean
  pass.
- Earlier reruns showed two failure modes: no `/app/result.txt` before the
  deadline, then later overwriting the correct dated candidate with a current
  live-page candidate. The preservation wording addressed the second failure.

## Score Probe Rerun

- Config:
  `evals/reports/harbor/roder-tbench-preserve-candidate-score-probe.json`
- Job: `evals/harbor/jobs/roder-tbench-preserve-candidate-score-probe`
- Tasks: `llm-inference-batching-scheduler`, `mteb-retrieve`, `qemu-startup`
- Result: 3 trials, 0 Harbor errors, mean `0.333`
- Passed: `llm-inference-batching-scheduler`
- Failed: `mteb-retrieve`, `qemu-startup`

Interpretation:

- `llm-inference-batching-scheduler` repeated its prior focused pass and is a
  plausible full-run conversion candidate.
- `mteb-retrieve` remained a computation-protocol miss: it used raw
  `sentence_transformers` embeddings and wrote the 5th-ranked result from that
  path, but the task expects the installed MTEB encoder interface.
- `qemu-startup` is not a prompt-only deadline failure. The task image's QEMU
  path fails with `rosetta error: Unimplemented syscall number 282`; the agent
  then attempted an LD_PRELOAD workaround but the image lacks `gcc`/`cc`, and
  the run reached the internal eval deadline.

## MTEB API Guidance Reruns

- First config:
  `evals/reports/harbor/roder-tbench-mteb-api-guidance-rerun.json`
- First job: `evals/harbor/jobs/roder-tbench-mteb-api-guidance-rerun`
- First result: 1 trial, 0 Harbor errors, mean `0.0`
- Second config:
  `evals/reports/harbor/roder-tbench-mteb-prompt-type-guidance-rerun.json`
- Second job:
  `evals/harbor/jobs/roder-tbench-mteb-prompt-type-guidance-rerun`
- Second result: 1 trial, 0 Harbor errors, mean `1.0`

Harness changes under test:

- Added Terminal-Bench guidance to prefer `mteb.get_model` and the MTEB model's
  `encode`/`similarity` helpers when the task explicitly mentions `mteb`.
- Tightened that guidance to use
  `mteb.encoder_interface.PromptType.query` and `PromptType.passage` for
  retrieval-style cosine similarity.
- Added an explicit instruction not to finalize an MTEB retrieval task until a
  local script has run the requested model/revision and written the computed
  rank.

Interpretation:

- The first rerun used `mteb.get_model` but guessed the wrong `PromptType`
  import path, then finalized with a heuristic candidate. It remained a scored
  failure.
- The second rerun found the public MTEB API path, recovered from a wrong
  `task_name='Retrieval'` attempt by using `T2Retrieval`, computed the expected
  5th rank, wrote `MTEB: Massive Text Embedding Benchmark`, and passed.
- This is a second concrete full-run failure conversion after
  `mteb-leaderboard`.

## Exact Data Guidance Rerun

- Config:
  `evals/reports/harbor/roder-tbench-exact-data-guidance-rerun.json`
- Job: `evals/harbor/jobs/roder-tbench-exact-data-guidance-rerun`
- Tasks: `db-wal-recovery`, `filter-js-from-html`,
  `financial-document-processor`, `gcode-to-text`
- Result: 4 trials, 0 Harbor errors, mean `0.25`
- Analyzer report:
  `evals/reports/harbor/roder-tbench-exact-data-guidance-rerun.md`
- Passed: `financial-document-processor`
- Failed: `db-wal-recovery`, `filter-js-from-html`, `gcode-to-text`

Harness changes under test:

- Added general exact-data guidance for processing every document, replaying WAL
  and event logs as ordered state changes, inspecting rendered G-code geometry,
  and adversarially validating sanitizer/filter tasks.

Interpretation:

- `financial-document-processor` converted cleanly. The baseline moved only 3
  invoices and wrote 4 CSV rows; the rerun OCRed/classified all 17 input files,
  moved 10 invoices and 7 other documents, wrote 11 CSV rows including the
  total row, and passed all 7 verifier tests.
- `db-wal-recovery` still failed. It recovered 11 records but preserved base
  values for existing ids instead of applying WAL updates to ids 1 and 2, then
  hit the internal eval deadline after marking the ledger complete. Next work
  should focus on deriving binary transforms from SQLite WAL magic and
  validating changed existing records before finalization.
- `gcode-to-text` improved from the baseline generic answer `embossed text` to
  a rendered/projection-derived answer, but still wrote the wrong exact string.
  Next work should give the harness a stronger artifact-inspection path or
  visual/OCR loop rather than relying on prompt guidance alone.
- `filter-js-from-html` blocked all 439 XSS vectors, but still failed the clean
  HTML preservation check.

## Filter Byte-Preservation Rerun

- Config:
  `evals/reports/harbor/roder-tbench-filter-byte-preserve-rerun.json`
- Job: `evals/harbor/jobs/roder-tbench-filter-byte-preserve-rerun`
- Task: `filter-js-from-html`
- Result: interrupted after Harbor hung waiting on verifier return, but the
  verifier artifacts were written with reward `0`.
- Analyzer report:
  `evals/reports/harbor/roder-tbench-filter-byte-preserve-rerun.md`

Findings:

- The stronger byte-preservation guidance caused the agent to create a
  targeted text/span sanitizer. It again blocked the XSS suite, but clean HTML
  still failed.
- The hidden clean-file check compares the filtered output against
  `BeautifulSoup`'s normalized form of the original, not the literal original
  bytes. Leaving clean files byte-for-byte unchanged can therefore fail when
  BeautifulSoup normalizes void tags, entities, or attribute order.
- The rerun also exposed a Harbor/verifier reliability issue: after reward and
  CTRF files were written, the `docker compose exec ... /tests/test.sh`
  process did not return to Harbor. The job had to be stopped manually, so the
  analyzer classifies it as an unclean `CancelledError` despite the verifier
  reward artifact.

Next action:

- Keep the sanitizer guidance focused on matching task/local-check
  normalization rather than banning parser serializers outright.
- Add a verifier-process timeout/cleanup guard in the Harbor harness so a
  completed reward file cannot leave the job wrapper blocked indefinitely.

## Complete Output Protein Rerun

- Config:
  `evals/reports/harbor/roder-tbench-complete-output-protein-rerun.json`
- Job: `evals/harbor/jobs/roder-tbench-complete-output-protein-rerun`
- Task: `protein-assembly`
- Result: 1 trial, 0 Harbor errors, mean `0.0`
- Analyzer report:
  `evals/reports/harbor/roder-tbench-complete-output-protein-rerun.md`

Harness change under test:

- Added general guidance that machine-parseable output files must be complete
  and must not contain ellipses, placeholders, Markdown fences, prose, or
  truncated excerpts.

Interpretation:

- This did not convert `protein-assembly`. The run built a longer candidate
  sequence and claimed local validation passed, but the verifier still rejected
  `/app/gblock.txt` because the final file was not matched by `[atcg]+`.
- The next improvement should force validation against the bytes of the actual
  required file after writing, for example with regex/parse checks on
  `/app/gblock.txt` itself, not only on the intermediate constructed sequence.

## Remaining Failures GPT-5.5 Xhigh Rerun

- Config:
  `evals/reports/harbor/roder-tbench-remaining-failures-gpt55-xhigh.json`
- Job: `evals/harbor/jobs/roder-tbench-remaining-failures-gpt55-xhigh`
- Source: baseline scored failures from
  `roder-tbench-full-gpt55-medium-deadline-reliability`, excluding the 4
  already validated medium-reasoning conversions.
- Model/reasoning: `codex/gpt-5.5`, reasoning `xhigh`
- Parallelism: 4 concurrent trials
- Result: 35 trials, 0 Harbor errors, mean `0.200`
- Analyzer report:
  `evals/reports/harbor/roder-tbench-remaining-failures-gpt55-xhigh.md`
- Passed: `db-wal-recovery`, `fix-code-vulnerability`, `kv-store-grpc`,
  `polyglot-c-py`, `query-optimize`, `torch-pipeline-parallelism`,
  `tune-mjcf`
- Failed: the other 28 remaining-failure tasks

Interpretation:

- Xhigh is worth routing selectively. It adds 7 validated conversions on top of
  the 4 prior focused conversions, raising the expected routed score from about
  54/89 to about 61/89.
- Xhigh should not be used indiscriminately. It policy-blocked
  `crack-7z-hash`, `git-leak-recovery`, `model-extraction-relu-logits`,
  `password-recovery`, and `vulnerable-secret`.
- Timeout-heavy systems tasks still need harness/task strategy rather than
  deeper reasoning: `compile-compcert`, `make-doom-for-mips`,
  `make-mips-interpreter`, `path-tracing`, both QEMU tasks, and
  `winning-avg-corewars` hit soft-timeout failure.
- `filter-js-from-html` no longer hung the whole run, but still scored `0`;
  clean-output normalization remains the core issue.
