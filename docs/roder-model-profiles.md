# Roder Model Profiles

Roder model profiles are data-backed runtime hints for provider-specific harness behavior: edit tool shape, schema policy, instruction overlay, reasoning defaults, parallel tool calls, and context thresholds.

Offline profile evals compare the same fixtures across built-in profile models:

```sh
RODER_EVAL_OUTPUT_DIR=/tmp/roder-evals \
  cargo run -p roder-cli -- eval run evals/fixtures/model-profiles --offline --profiles all
```

The report includes `Model Profile Deltas` rows keyed by fixture and profile. Treat those rows as recommendation evidence only when a profile improves a failure class or keeps quality equivalent while reducing wall time, model calls, or tool calls.
