# Roder Model Profiles

Roder model profiles are data-backed runtime hints for provider-specific harness behavior: edit tool shape, schema policy, instruction overlay, reasoning defaults, parallel tool calls, and context thresholds.

Offline profile evals compare the same fixtures across built-in profile models:

```sh
RODER_EVAL_OUTPUT_DIR=/tmp/roder-evals \
  cargo run -p roder -- eval run evals/fixtures/model-profiles --offline --profiles all
```

The report includes `Model Profile Deltas` rows keyed by fixture and profile. Treat those rows as recommendation evidence only when a profile improves a failure class or keeps quality equivalent while reducing wall time, model calls, or tool calls.

Image generation models (OpenAI `gpt-image-*`, Google `gemini-*-image`) are not chat models: they live in the separate media image catalog, never appear in chat model pickers or harness profiles, and are configured through `[media.image_generation]` (see `docs/roder-image-generation-providers.md`).
