# Local Usage Analytics

Roder records local long-term usage analytics — token spend, tool
popularity, latency percentiles, errors, and session summaries — into a
SQLite database under the Roder data directory:

```text
<data-dir>/analytics/usage.sqlite3
```

Analytics are **local-only**: nothing is uploaded anywhere, and there is no
remote telemetry path. Raw per-thread `events.jsonl` files remain the
durable audit stream; the analytics store is a queryable projection that can
always be rebuilt from them.

## Privacy defaults

- Recorded: tool names, provider/model ids, workspace labels, thread/turn
  ids, timestamps, status, durations, token counts, bounded error classes.
- Never recorded: prompt bodies, assistant text, tool output bodies, command
  payloads, API keys, bearer tokens, environment secrets.
- Workspace paths are local metadata. `workspace_labels` controls what is
  stored and reported: `full_path` (default), `hashed` (FNV-1a), or
  `basename_only`.

```toml
[analytics]
enabled = true                 # default; set false to record nothing
workspace_labels = "full_path" # or "hashed" / "basename_only"
retention_days = 0             # 0 keeps everything; N prunes raw rows older than N days
# store = "/custom/path/usage.sqlite3"
```

Retention pruning runs once per process start and during `stats backfill`;
sessions are kept while any of their turns or tool calls remain.

Recording is passive: the analytics recorder is an event sink behind the
runtime's bounded per-sink dispatch, so a slow or broken store can never
block tool execution, provider requests, or turn progress. Disabling
analytics removes the sink entirely.

## Backfill

Existing JSONL thread stores import idempotently:

```sh
roder stats backfill                 # incremental (import offsets skip old lines)
roder stats backfill --rebuild       # clear analytics rows and replay everything
roder stats backfill --best-effort   # report corrupt lines (file:line) and continue
```

## Example queries

```sh
roder stats summary --since 7d --json
roder stats tools --sort p95               # p50/p95/p99 per tool
roder stats tools --sort errors            # highest error rates
roder stats tools --sort underused         # least-called tools
roder stats tokens --group day             # token spend by day
roder stats tokens --group session --json  # expensive sessions
roder stats sessions                       # tool/turn/error/token totals per thread
roder stats export --format jsonl --output usage-export.jsonl
```

All commands accept `--data-dir <dir>` for fixtures/tests, `--since`/
`--until` windows (`7d`, `24h`, `90m`, `YYYY-MM-DD`), and `--json` for
stable machine-readable output. Percentiles are exact (nearest-rank over
raw durations); `daily_rollups` caches per-day aggregates for dashboards
and is refreshed by `stats backfill`.

## App-server methods

`stats/summary`, `stats/tools`, `stats/tokens`, `stats/sessions`,
`stats/backfill`, and `stats/export` mirror the CLI query path. Result
limits are bounded server-side (hard cap 1000 rows; larger explicit limits
return a typed validation error), workspace labels are transformed by the
configured mode before leaving the store, and `stats/export` writes a
server-side artifact rather than streaming an unbounded payload.

## Testing

```sh
cargo test -p roder-usage-analytics
cargo test -p roder-core --test usage_analytics
cargo test -p roder-cli --bin roder stats
cargo test -p roder-app-server stats
```

Everything runs offline with temp data dirs and fake events; no provider
credentials or network access.
