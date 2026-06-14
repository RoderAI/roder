# Roder Speed Policy

Roder's speed policy is an eval/headless runtime policy for reducing wall-clock time without hiding verification quality. It records the current turn phase, applies the lowest useful reasoning budget for that phase, keeps subagent fanout bounded, and carries turn deadline information into eval traces and reports.

## Running Speed Evals

Run the canonical offline speed suite with speed policy enabled:

```sh
RODER_EVAL_OUTPUT_DIR=/tmp/roder-evals cargo run -p roder -- eval run evals/fixtures/speed --offline --speed-policy on
```

Run baseline and speed policy in one report:

```sh
RODER_EVAL_OUTPUT_DIR=/tmp/roder-evals-speed-both cargo run -p roder -- eval run evals/fixtures/speed --offline --speed-policy both
```

The generated `eval-report.md` includes:

- wall time
- model-call count
- tool-call count
- child-task count
- remaining turn deadline
- pass/fail outcome
- baseline-vs-speed comparison rows when `--speed-policy both` is used

## Tuning Thresholds

Tune speed policy in config and eval inputs before changing defaults in code:

```toml
[speed_policy]
enabled = true
orientation_assistant_messages = 10
orientation_reasoning = "high"
execution_reasoning = "low"
verification_reasoning = "high"
recovery_reasoning = "medium"
max_parallel_subagents = 4
subagent_timeout_seconds = 120
eval_deadline_seconds = 600
```

Use lower execution reasoning only after the orientation call has enough context to choose the right path. Increase `orientation_assistant_messages` when eval reports show early wrong-file reads or verifier regressions. Decrease it only when reports show identical pass rates and lower wall time.

Use `--speed-policy both` after every threshold change. A candidate threshold is not ready to become a default unless speed rows improve or match wall time and the comparison quality column still matches the baseline outcome for every required fixture.
