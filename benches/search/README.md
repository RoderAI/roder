# Search Benchmark Harness

Run the benchmark against the built-in synthetic monorepo fixture:

```sh
cargo bench -p roder-search --bench search
```

Or run it against an external repository fixture:

```sh
RODER_SEARCH_BENCH_FIXTURE=/path/to/repo \
RODER_SEARCH_BENCH_ITERATIONS=15 \
RODER_SEARCH_BENCH_QUERIES='fn ,struct ,Config' \
cargo bench -p roder-search --bench search
```

The harness prints scan and indexed p50/p90/p99 latency, match counts, candidate files, verified files, index bytes, and warmup build time. If `RODER_SEARCH_BENCH_FIXTURE` is unset, it creates a temporary synthetic monorepo fixture under the system temp directory.

## Evidence From Real Repositories

Captured locally on 2026-05-20 with shallow clones under `~/tmp`.

### `~/tmp/roder-bench-ripgrep`

Command:

```sh
RODER_SEARCH_BENCH_FIXTURE="$HOME/tmp/roder-bench-ripgrep" \
RODER_SEARCH_BENCH_ITERATIONS=15 \
RODER_SEARCH_BENCH_QUERIES='fn ,struct ,Regex' \
cargo bench -p roder-search --bench search
```

Results:

```text
query,engine,matches,candidate_files,verified_files,index_bytes,build_ms,p50_ms,p90_ms,p99_ms
fn,scan,2822,210,210,0,0,9.685,11.124,11.615
fn,indexed,2822,210,210,2484157,87,9.292,9.825,9.893
struct,scan,424,210,210,0,0,8.606,9.397,9.614
struct,indexed,424,64,64,2484157,98,2.568,2.912,3.057
Regex,scan,375,210,210,0,0,8.391,9.117,9.400
Regex,indexed,375,72,72,2484157,94,2.361,2.883,3.152
```

Selective query p50 wins: `struct` 3.35x, `Regex` 3.55x.

### `~/tmp/roder-bench-rustfmt`

Command:

```sh
RODER_SEARCH_BENCH_FIXTURE="$HOME/tmp/roder-bench-rustfmt" \
RODER_SEARCH_BENCH_ITERATIONS=15 \
RODER_SEARCH_BENCH_QUERIES='fn ,struct ,Config' \
cargo bench -p roder-search --bench search
```

Results:

```text
query,engine,matches,candidate_files,verified_files,index_bytes,build_ms,p50_ms,p90_ms,p99_ms
fn,scan,3663,1021,1021,0,0,23.719,25.243,31.816
fn,indexed,3663,1021,1021,3322598,110,24.250,25.622,26.003
struct,scan,811,1021,1021,0,0,21.910,24.411,25.761
struct,indexed,811,203,203,3322598,103,3.906,5.061,5.111
Config,scan,433,1021,1021,0,0,22.697,24.642,25.670
Config,indexed,433,109,109,3322598,105,2.860,3.547,3.941
```

Selective query p50 wins: `struct` 5.61x, `Config` 7.93x.

The broad `fn` query is intentionally included as a control: it appears in most files, so trigram candidate narrowing cannot reduce verification work much.
