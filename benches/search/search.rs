use std::env;
use std::hint::black_box;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use roder_search::{DEFAULT_MAX_FILE_SIZE, SearchMode, SearchOptions, WorkspaceSearcher};

fn main() {
    let Some(fixture) = env::var_os("RODER_SEARCH_BENCH_FIXTURE").map(PathBuf::from) else {
        println!("skipping search benchmark: set RODER_SEARCH_BENCH_FIXTURE=/path/to/repo");
        return;
    };
    if !fixture.exists() {
        println!(
            "skipping search benchmark: fixture does not exist: {}",
            fixture.display()
        );
        return;
    }

    let iterations = env::var("RODER_SEARCH_BENCH_ITERATIONS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(25)
        .max(3);
    let queries = env::var("RODER_SEARCH_BENCH_QUERIES")
        .unwrap_or_else(|_| "fn ,struct ,TODO".to_string())
        .split(',')
        .map(str::trim)
        .filter(|query| !query.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();

    println!("fixture: {}", fixture.display());
    println!("iterations: {iterations}");
    println!(
        "query,engine,matches,candidate_files,verified_files,index_bytes,build_ms,p50_ms,p90_ms,p99_ms"
    );

    for query in queries {
        let scan = measure_scan(&fixture, &query, iterations);
        let indexed = measure_indexed(&fixture, &query, iterations);
        print_report(&query, "scan", &scan);
        print_report(&query, "indexed", &indexed);
        if indexed.p50 < scan.p50 {
            let improvement =
                scan.p50.as_secs_f64() / indexed.p50.max(Duration::from_nanos(1)).as_secs_f64();
            println!("improvement,{query},p50,{improvement:.2}x");
        }
    }
}

#[derive(Clone, Debug)]
struct Report {
    matches: usize,
    candidate_files: usize,
    verified_files: usize,
    index_bytes: Option<u64>,
    build_ms: Option<u128>,
    p50: Duration,
    p90: Duration,
    p99: Duration,
}

fn measure_scan(root: &PathBuf, query: &str, iterations: usize) -> Report {
    let mut timings = Vec::with_capacity(iterations);
    let mut last = None;
    for _ in 0..iterations {
        let mut options = options(query);
        options.mode = SearchMode::Scan;
        let started = Instant::now();
        let output = roder_search::search_workspace(root, &options).expect("scan search succeeds");
        timings.push(started.elapsed());
        black_box(&output.lines);
        last = Some(output);
    }
    report(timings, last.expect("at least one iteration"))
}

fn measure_indexed(root: &PathBuf, query: &str, iterations: usize) -> Report {
    let mut options = options(query);
    options.mode = SearchMode::Indexed;
    let mut searcher = WorkspaceSearcher::new(root);
    let warm_started = Instant::now();
    searcher.warm(&options).expect("index warmup succeeds");
    let warm_ms = warm_started.elapsed().as_millis();

    let mut timings = Vec::with_capacity(iterations);
    let mut last = None;
    for _ in 0..iterations {
        let started = Instant::now();
        let output = searcher.search(&options).expect("indexed search succeeds");
        timings.push(started.elapsed());
        black_box(&output.lines);
        last = Some(output);
    }
    let mut report = report(timings, last.expect("at least one iteration"));
    report.build_ms = Some(warm_ms);
    report
}

fn options(query: &str) -> SearchOptions {
    let mut options = SearchOptions::new(query);
    options.path = PathBuf::from(".");
    options.regex = false;
    options.case_sensitive = true;
    options.word_boundary = false;
    options.max_file_size = DEFAULT_MAX_FILE_SIZE;
    options
}

fn report(mut timings: Vec<Duration>, output: roder_search::SearchResults) -> Report {
    timings.sort();
    Report {
        matches: output.lines.len(),
        candidate_files: output.metadata.candidate_files,
        verified_files: output.metadata.verified_files,
        index_bytes: output.metadata.index_bytes,
        build_ms: output.metadata.index_build_time_ms,
        p50: percentile(&timings, 50),
        p90: percentile(&timings, 90),
        p99: percentile(&timings, 99),
    }
}

fn percentile(timings: &[Duration], percentile: usize) -> Duration {
    let index = ((timings.len() - 1) * percentile).div_ceil(100);
    timings[index]
}

fn print_report(query: &str, engine: &str, report: &Report) {
    println!(
        "{query},{engine},{},{},{},{},{},{:.3},{:.3},{:.3}",
        report.matches,
        report.candidate_files,
        report.verified_files,
        report.index_bytes.unwrap_or_default(),
        report.build_ms.unwrap_or_default(),
        millis(report.p50),
        millis(report.p90),
        millis(report.p99)
    );
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}
