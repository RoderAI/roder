# Roder Reliability Guardrails

Roder records reliability failures as classified local events and eval metrics. The focused reliability suite is local-only and uses fake provider/tool scenarios so it can run before release without live provider credentials.

Run the focused suite:

```sh
RODER_EVAL_OUTPUT_DIR=/tmp/roder-evals-reliability \
  roder eval run evals/fixtures/reliability --offline
```

Compare the latest report against the checked-in baseline:

```sh
RODER_EVAL_OUTPUT_DIR=/tmp/roder-evals-reliability \
  roder eval report --compare-baseline evals/baselines/reliability.json
```

The baseline format is JSON:

```json
{
  "version": 1,
  "unknownErrorBlockerThreshold": 0,
  "expectations": [
    {
      "scope": "model:mock/mock",
      "metric": "reliability_error_class_provider_error",
      "maxCount": 2,
      "maxIncrease": 1
    }
  ]
}
```

Scopes are `suite`, `model:<provider>/<model>`, or an existing fixture tag such as `tool:read_file`. Unknown errors above the blocker threshold are release blockers. Other expected error classes use `maxCount + maxIncrease` to flag local spikes for follow-up.

Pre-release workflow:

```sh
out=/tmp/roder-evals-reliability-$(date +%Y%m%d)
RODER_EVAL_OUTPUT_DIR="$out" roder eval run evals/fixtures/reliability --offline
RODER_EVAL_OUTPUT_DIR="$out" roder eval report --compare-baseline evals/baselines/reliability.json > "$out/reliability-backlog.md"
```

Use the generated comparison rows as the backlog seed. Keep the baseline in-repo and update it only when the current behavior is intentionally accepted.
