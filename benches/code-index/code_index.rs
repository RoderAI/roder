use std::env;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::Instant;

use roder_api::code_index::CodeIndexSearchRequest;
use roder_code_index::sqlite::SqliteCodeIndexStore;

fn main() {
    let fixture = env::var_os("RODER_CODE_INDEX_BENCH_FIXTURE")
        .map(PathBuf::from)
        .unwrap_or_else(generate_synthetic_fixture);
    let store_path = env::temp_dir().join(format!(
        "roder-code-index-bench-{}-code-index.sqlite3",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&store_path);
    let store = SqliteCodeIndexStore::open(&store_path).expect("open code-index bench store");

    let full_started = Instant::now();
    let full = store
        .rebuild_workspace(&fixture)
        .expect("full code-index rebuild succeeds");
    let full_ms = full_started.elapsed().as_millis();

    let first_query_started = Instant::now();
    let first = store
        .search(CodeIndexSearchRequest {
            query_id: "bench-first-query".to_string(),
            query: "oauth refresh token".to_string(),
            workspace_root: fixture.clone(),
            limit: 5,
        })
        .expect("first code-index query succeeds");
    let first_query_ms = first_query_started.elapsed().as_micros() as f64 / 1000.0;
    black_box(&first);

    mutate_one_file(&fixture);
    let incremental_started = Instant::now();
    let incremental = store
        .rebuild_workspace(&fixture)
        .expect("incremental code-index rebuild succeeds");
    let incremental_ms = incremental_started.elapsed().as_millis();
    let cache_hit_rate = if incremental.generation.stats.chunk_count == 0 {
        0.0
    } else {
        incremental.generation.stats.cached_embedding_count as f64
            / incremental.generation.stats.chunk_count as f64
    };

    println!("fixture: {}", fixture.display());
    println!(
        "metric,value\nfull_build_ms,{full_ms}\nfull_chunk_count,{}\nfirst_query_ms,{first_query_ms:.3}\nincremental_update_ms,{incremental_ms}\nchanged_files,{}\nreused_files,{}\nchunk_count,{}\nembedded_chunks,{}\ncached_chunks,{}\ncache_hit_rate,{cache_hit_rate:.3}\nfirst_query_results,{}",
        full.generation.stats.chunk_count,
        incremental.changed_file_count,
        incremental.reused_file_count,
        incremental.generation.stats.chunk_count,
        incremental.generation.stats.embedded_chunk_count,
        incremental.generation.stats.cached_embedding_count,
        first.results.len(),
    );
}

fn generate_synthetic_fixture() -> PathBuf {
    let root = env::temp_dir().join(format!(
        "roder-code-index-synthetic-bench-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("create synthetic code-index fixture");
    for crate_index in 0..8 {
        let src = root.join(format!("crates/service-{crate_index:02}/src"));
        std::fs::create_dir_all(&src).expect("create synthetic crate");
        for file_index in 0..18 {
            let symbol = if crate_index == 4 && file_index == 9 {
                "oauth_refresh_token"
            } else {
                "ordinary_handler"
            };
            std::fs::write(
                src.join(format!("module_{file_index:02}.rs")),
                format!(
                    "pub fn {symbol}_{crate_index}_{file_index}() -> &'static str {{\n    \"semantic-index-fixture\"\n}}\n"
                ),
            )
            .expect("write synthetic source");
        }
    }
    root
}

fn mutate_one_file(root: &Path) {
    let path = root.join("crates/service-04/src/module_09.rs");
    std::fs::write(
        path,
        "pub fn oauth_refresh_token_changed() -> &'static str {\n    \"changed\"\n}\n",
    )
    .expect("mutate one synthetic file");
}
