# Roder Evals

Roder evals run local fixture suites through the Rust runtime and write two files:

- `eval-run.json`: structured run, fixture, trajectory, metric, and failure data.
- `eval-report.md`: bounded human-readable summary with failure groups by tool, model, and failure class.

Normal contributor runs are offline and use the deterministic `mock/mock` provider:

```sh
RODER_EVAL_OUTPUT_DIR=/tmp/roder-evals roder eval run evals/fixtures --offline
roder eval list --output-dir /tmp/roder-evals
roder eval report eval-run --output-dir /tmp/roder-evals --max-bytes 65536
```

If `--output-dir` is omitted, the CLI uses `RODER_EVAL_OUTPUT_DIR`, then `evals/reports`.

Live-provider mode is guarded so it cannot run by accident. A live run must set `RODER_EVAL_LIVE_PROVIDER=1` and pass explicit `--provider` and `--model` values. This phase wires the guard and the mock path:

```sh
RODER_EVAL_LIVE_PROVIDER=1 roder eval run evals/fixtures --provider mock --model mock
```

App-server clients can inspect recent reports without arbitrary file reads:

```json
{ "jsonrpc": "2.0", "id": 1, "method": "eval/reports/list", "params": { "limit": 20 } }
```

```json
{ "jsonrpc": "2.0", "id": 2, "method": "eval/report/read", "params": { "reportId": "eval-run", "maxBytes": 65536 } }
```

The app-server only resolves report ids returned by `eval/reports/list` under the workspace `evals/reports` directory.
