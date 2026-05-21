# Roder Planning And Verification Gates

Roder's eval runtime profile is for unattended runs. It keeps chat-friendly behavior out of benchmark and headless turns:

- `request_user_input` returns a model-visible unavailable result instead of waiting for a user.
- Decomposed eval fixtures can set `expected.taskLedgerRequired = true`; missing, stale, or incomplete task ledgers fail the eval.
- Code-changing eval turns must call `verification.review` before final completion.

Run the local verification suite with:

```sh
RODER_EVAL_OUTPUT_DIR=/tmp/roder-evals \
  cargo run -p roder-cli -- eval run evals/fixtures/verification --offline --profile eval
```

The report includes task-ledger and verification tables. Verification fields show whether the gate was required, completed, failed, skipped, and whether any open gaps remain.

Interactive users keep manual control by default. Use the default `interactive` profile for normal TUI/app-server sessions, or choose the stricter behavior explicitly with `--profile eval`, `runtime_profile = "eval"`, or `RODER_RUNTIME_PROFILE=eval`.

The verification tool receives:

- `originalTask`
- `changedFiles`
- `toolEvidence`
- `testsRun`
- `openGaps`
- `status`: `completed`, `failed`, or `skipped`

Skipped verification is only appropriate for read-only, question-answering, or no-op turns. Code-changing eval turns are blocked until verification completes without open gaps.
