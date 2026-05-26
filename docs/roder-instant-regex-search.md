# Roder Instant Regex Search

Roder's canonical `grep` tool supports literal and regex code search through a local search engine. The indexed path is an acceleration layer, not a correctness requirement: every result returned to the model is verified against current file contents before it is shown.

## Tool Contract

The model-facing tool remains `grep`. It accepts:

```json
{
  "query": "Result<.*Error>",
  "path": "crates",
  "regex": true,
  "case_sensitive": true,
  "word_boundary": false,
  "offset": 0,
  "limit": 200,
  "mode": "auto"
}
```

`mode` may be:

- `auto`: use the local index when it can narrow the query, otherwise scan.
- `indexed`: prefer indexed candidate lookup, then verify matches; broad or unsupported queries safely fall back.
- `scan`: traverse files and verify directly without using the index.

Result text is still formatted as `relative/path:line:content`. `ToolResult.data` includes pagination plus search metadata:

```json
{
  "engine": "indexed",
  "index_version": "fastregex-v1",
  "candidate_files": 42,
  "verified_files": 12,
  "stale": false,
  "elapsed_ms": 8
}
```

## Index Behavior

The index records searchable text files under the workspace root, excluding `.git`, `target`, common binary files, and files above the configured size limit. It stores n-gram postings that can identify candidate files for many literal and regex queries. Candidate files are always re-read and verified before output, so broad query decomposition can only cost extra verification work; it cannot create false final matches.

Remote runner workspaces initially use fallback scanning. Local index storage is rooted under `~/.roder/indexes/<workspace-id>/`; test and harness runs can override the home root with `RODER_SEARCH_INDEX_HOME`.

## App-Server Inspection

App clients and palette surfaces can inspect and manage the persistent index through JSON-RPC:

- `search_index/status` returns the current index state without reading private files.
- `search_index/warmup` builds the index when it is missing, or reports the existing status when it is already present.
- `search_index/rebuild` explicitly rebuilds the persistent index for the workspace.
- `search_index/clear` removes the persistent index cache for the workspace.

Each method accepts `{ "workspace": "/path/to/workspace" }`; omitting `workspace` uses the app-server runtime workspace. Responses include a `status` object with `state`, `enabled`, `workspace`, `storeDir`, optional `indexVersion`, optional document/byte/build metrics, a `stale` flag, and an optional message.

Status states are:

- `disabled`: indexed search is off through config or environment.
- `missing`: no persistent index has been built for the workspace.
- `building`: a warmup or rebuild is in progress.
- `ready`: the index is present and matches current file metadata.
- `stale`: the index exists but at least one indexed document changed, disappeared, or no longer matches the workspace/index version.
- `failed`: status inspection, rebuild, or clear failed.
- `cleared`: the index cache was removed.

Warmup, rebuild, clear, and settings toggles publish `search_index/statusChanged` notifications so TUI and palette surfaces can show readable progress without polling. The notification payload is `{ "status": ... }` using the same shape as the method responses.

The indexed path is always a performance optimization. If the status is `disabled`, `missing`, `stale`, or `failed`, `grep` remains correct by using fallback scanning or by verifying candidate files against current file contents before returning matches.

## Configuration

`~/.roder/config.toml` may include:

```toml
[search_index]
enabled = true
max_file_bytes = 1048576
ignored_globs = ["vendor/**", "*.min.js"]
rebuild_concurrency = 4
max_index_bytes = 536870912
```

You can also toggle this from the TUI:

1. Press `Ctrl+P`.
2. Open `Settings`.
3. Select `Instant regex search: on/off`.

The setting is applied immediately for new `grep` calls and persisted to `~/.roder/config.toml` when the TUI app-server is running with user config persistence.

Environment overrides:

- `RODER_SEARCH_INDEX_DISABLED=1` disables indexed search.
- `RODER_SEARCH_INDEX_MAX_FILE_BYTES=2097152` changes the per-file indexing limit.

## Benchmarks

Correctness tests compare indexed search with full scans on temporary workspaces. Latency benchmarks are opt-in for large repositories and should be run with a fixture path:

```sh
RODER_SEARCH_BENCH_FIXTURE=/path/to/repo cargo bench -p roder-search --bench search
```

The benchmark report compares scan and indexed search, including p50/p90/p99 latency, candidate files, verified files, build time, and index size. Large-repo fixtures should live outside the project checkout, commonly under `~/tmp`, so benchmark data and cloned repositories are not committed.
