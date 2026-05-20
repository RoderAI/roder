# Fastregex Search Iteration - 2026-05-20

## Findings

- The first implementation correctly accelerated selective queries, but the cached `WorkspaceSearcher` could be warmed for a narrow `path` and then reused for a later query with a different path. That risked false negatives in long-lived Roder sessions.
- The local `grep` backend also kept a warm index after `write_file`, `edit`, `multi_edit`, and `apply_patch`, so searches after Roder-authored edits could miss newly added matching files.
- Broad queries such as `fn` are not expected to improve much because their trigrams appear in most files, so candidate verification stays close to a full scan.

## Changes Made

- `WorkspaceSearcher::warm` now builds a workspace-wide index, independent of the first query path.
- Indexed search now filters candidate files by the current query path before verification, preserving `grep path=...` semantics.
- `WorkspaceSearcher::invalidate` was added and the local tools backend clears the warm index after writes, edits, multi-edits, and patches.
- Added regression tests for path-scoped cache reuse and post-write index refresh.

## Validation

```sh
cargo test -p roder-search cached_workspace_index_still_respects_later_query_paths
cargo test -p roder-tools grep_refreshes_index
cargo test -p roder-tools grep_supports_regex
```

All three passed.

## Benchmarks After Iteration

Benchmarks use shallow real-repo clones under `~/tmp`, 15 iterations per query.

### `~/tmp/roder-bench-ripgrep`

Command:

```sh
RODER_SEARCH_BENCH_FIXTURE="$HOME/tmp/roder-bench-ripgrep" \
RODER_SEARCH_BENCH_ITERATIONS=15 \
RODER_SEARCH_BENCH_QUERIES='fn ,struct ,Regex' \
cargo bench -p roder-search --bench search
```

Output:

```text
query,engine,matches,candidate_files,verified_files,index_bytes,build_ms,p50_ms,p90_ms,p99_ms
fn,scan,2822,210,210,0,0,8.467,9.498,32.562
fn,indexed,2822,210,210,2484157,87,8.211,9.136,9.168
improvement,fn,p50,1.03x
struct,scan,424,210,210,0,0,7.524,8.378,8.378
struct,indexed,424,64,64,2484157,90,2.195,2.951,3.184
improvement,struct,p50,3.43x
Regex,scan,375,210,210,0,0,7.117,7.926,7.935
Regex,indexed,375,72,72,2484157,84,2.088,2.871,2.905
improvement,Regex,p50,3.41x
```

### `~/tmp/roder-bench-rustfmt`

Command:

```sh
RODER_SEARCH_BENCH_FIXTURE="$HOME/tmp/roder-bench-rustfmt" \
RODER_SEARCH_BENCH_ITERATIONS=15 \
RODER_SEARCH_BENCH_QUERIES='fn ,struct ,Config' \
cargo bench -p roder-search --bench search
```

Output:

```text
query,engine,matches,candidate_files,verified_files,index_bytes,build_ms,p50_ms,p90_ms,p99_ms
fn,scan,3663,1021,1021,0,0,24.066,24.826,98.632
fn,indexed,3663,1021,1021,3322598,109,24.244,25.434,25.587
struct,scan,811,1021,1021,0,0,22.257,23.583,24.272
struct,indexed,811,203,203,3322598,107,4.169,5.443,5.520
improvement,struct,p50,5.34x
Config,scan,433,1021,1021,0,0,22.053,23.172,23.892
Config,indexed,433,109,109,3322598,107,2.499,3.164,3.425
improvement,Config,p50,8.83x
```

## Remaining Improvement Ideas

- Add file watching or cheap workspace generation tracking so external editor changes can invalidate the warm index too.
- Persist postings to disk and mmap lookup tables to remove warmup cost across Roder process restarts.
- Add a query planner that estimates candidate selectivity and chooses scan for broad indexed candidates such as `fn`.
- Extend regex decomposition for simple alternations like `foo|bar` by unioning literal-run candidate sets before verification.
