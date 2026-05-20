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

Remote runner workspaces initially use fallback scanning. Local index storage is rooted under `~/.roder/indexes/<workspace-id>/` as persistent storage is expanded.

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
