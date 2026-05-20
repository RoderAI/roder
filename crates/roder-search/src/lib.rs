mod index;
mod postings;
mod query;
mod store;

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

pub use index::{Document, IndexStats, SearchIndex};
use index::{collect_text_files, scoped_path};
use postings::FileId;
use query::CompiledQuery;
pub use store::{
    IndexStoreMetadata, STORE_MANIFEST_FILE, default_store_dir, manifest_path, workspace_key,
};

pub const INDEX_VERSION: &str = "roder-search-v1";
pub const DEFAULT_MAX_FILE_SIZE: u64 = 2 * 1024 * 1024;

static SEARCH_INDEX_ENABLED: AtomicBool = AtomicBool::new(true);

pub fn set_search_index_enabled(enabled: bool) {
    SEARCH_INDEX_ENABLED.store(enabled, Ordering::SeqCst);
}

pub fn search_index_enabled() -> bool {
    SEARCH_INDEX_ENABLED.load(Ordering::SeqCst)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchMode {
    Auto,
    Indexed,
    Scan,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchEngine {
    Indexed,
    Scan,
    Fallback,
}

impl SearchEngine {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Indexed => "indexed",
            Self::Scan => "scan",
            Self::Fallback => "fallback",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchOptions {
    pub query: String,
    pub path: PathBuf,
    pub mode: SearchMode,
    pub regex: bool,
    pub case_sensitive: bool,
    pub word_boundary: bool,
    pub max_file_size: u64,
}

impl SearchOptions {
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            path: PathBuf::from("."),
            mode: SearchMode::Auto,
            regex: false,
            case_sensitive: true,
            word_boundary: false,
            max_file_size: DEFAULT_MAX_FILE_SIZE,
        }
    }

    pub fn with_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.path = path.into();
        self
    }

    pub fn with_mode(mut self, mode: SearchMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn regex(mut self, regex: bool) -> Self {
        self.regex = regex;
        self
    }

    pub fn case_sensitive(mut self, case_sensitive: bool) -> Self {
        self.case_sensitive = case_sensitive;
        self
    }

    pub fn word_boundary(mut self, word_boundary: bool) -> Self {
        self.word_boundary = word_boundary;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchLine {
    pub path: PathBuf,
    pub line: usize,
    pub content: String,
}

impl SearchLine {
    pub fn formatted(&self) -> String {
        format!("{}:{}:{}", slash_path(&self.path), self.line, self.content)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchMetadata {
    pub engine: SearchEngine,
    pub candidate_files: usize,
    pub verified_files: usize,
    pub stale: bool,
    pub elapsed_ms: u128,
    pub index_version: String,
    pub index_bytes: Option<u64>,
    pub index_build_time_ms: Option<u128>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchResults {
    pub lines: Vec<String>,
    pub matches: Vec<SearchLine>,
    pub metadata: SearchMetadata,
}

#[derive(Debug)]
pub enum SearchError {
    Io(std::io::Error),
    InvalidPath(String),
    InvalidQuery(String),
}

impl fmt::Display for SearchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(formatter, "{err}"),
            Self::InvalidPath(message) | Self::InvalidQuery(message) => {
                formatter.write_str(message)
            }
        }
    }
}

impl std::error::Error for SearchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::InvalidPath(_) | Self::InvalidQuery(_) => None,
        }
    }
}

impl From<std::io::Error> for SearchError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

pub fn search_workspace(
    root: impl AsRef<Path>,
    options: &SearchOptions,
) -> Result<SearchResults, SearchError> {
    WorkspaceSearcher::new(root).search(options)
}

#[derive(Clone, Debug)]
pub struct WorkspaceSearcher {
    root: PathBuf,
    index: Option<SearchIndex>,
}

impl WorkspaceSearcher {
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
            index: None,
        }
    }

    pub fn warm(&mut self, options: &SearchOptions) -> Result<&SearchIndex, SearchError> {
        let mut workspace_options = options.clone();
        workspace_options.path = PathBuf::from(".");
        self.index = Some(SearchIndex::build(&self.root, &workspace_options)?);
        Ok(self.index.as_ref().expect("index was just built"))
    }

    pub fn index(&self) -> Option<&SearchIndex> {
        self.index.as_ref()
    }

    pub fn invalidate(&mut self) {
        self.index = None;
    }

    pub fn search(&mut self, options: &SearchOptions) -> Result<SearchResults, SearchError> {
        let start = Instant::now();
        let query = CompiledQuery::compile(options)?;

        match options.mode {
            SearchMode::Scan => {
                scan_search(&self.root, options, &query, start, SearchEngine::Scan, None)
            }
            SearchMode::Auto | SearchMode::Indexed if !search_index_enabled() => {
                scan_search(&self.root, options, &query, start, SearchEngine::Scan, None)
            }
            SearchMode::Auto if query.required_trigrams().is_empty() => {
                scan_search(&self.root, options, &query, start, SearchEngine::Scan, None)
            }
            SearchMode::Auto | SearchMode::Indexed => {
                if self.index.is_none() {
                    self.warm(options)?;
                }
                let index = self.index.as_ref().expect("index is initialized");
                let stats = index.stats().clone();
                let Some(candidate_ids) = index.candidate_file_ids(&query) else {
                    return scan_search(
                        &self.root,
                        options,
                        &query,
                        start,
                        SearchEngine::Fallback,
                        Some(&stats),
                    );
                };

                indexed_search(index, options, &query, candidate_ids, start)
            }
        }
    }
}

fn scan_search(
    root: &Path,
    options: &SearchOptions,
    query: &CompiledQuery,
    start: Instant,
    engine: SearchEngine,
    index_stats: Option<&IndexStats>,
) -> Result<SearchResults, SearchError> {
    let scope = scoped_path(root, &options.path)?;
    let files = collect_text_files(root, &scope, options.max_file_size)?;
    let verified_files = files.len();
    let mut matches = Vec::new();

    for file in files {
        append_matches(&file.relative_path, &file.content, query, &mut matches);
    }

    Ok(finalize_results(
        matches,
        SearchMetadata {
            engine,
            candidate_files: verified_files,
            verified_files,
            stale: false,
            elapsed_ms: start.elapsed().as_millis(),
            index_version: INDEX_VERSION.to_string(),
            index_bytes: index_stats.map(|stats| stats.index_bytes),
            index_build_time_ms: index_stats.map(|stats| stats.build_time_ms),
        },
    ))
}

fn indexed_search(
    index: &SearchIndex,
    options: &SearchOptions,
    query: &CompiledQuery,
    candidate_ids: BTreeSet<FileId>,
    start: Instant,
) -> Result<SearchResults, SearchError> {
    let scope = scoped_path(index.root(), &options.path)?;
    let mut matches = Vec::new();
    let mut stale = false;
    let mut verified_files = 0;
    let mut scoped_candidate_files = 0;

    for id in &candidate_ids {
        let Some(path) = index.document_path(*id) else {
            stale = true;
            continue;
        };
        if !path.starts_with(&scope) {
            continue;
        }
        scoped_candidate_files += 1;
        if index.document_is_stale(*id) {
            stale = true;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            stale = true;
            continue;
        };
        verified_files += 1;

        let relative = path
            .strip_prefix(index.root())
            .unwrap_or(&path)
            .to_path_buf();
        append_matches(&relative, &content, query, &mut matches);
    }

    Ok(finalize_results(
        matches,
        SearchMetadata {
            engine: SearchEngine::Indexed,
            candidate_files: scoped_candidate_files,
            verified_files,
            stale,
            elapsed_ms: start.elapsed().as_millis(),
            index_version: index.stats().index_version.clone(),
            index_bytes: Some(index.stats().index_bytes),
            index_build_time_ms: Some(index.stats().build_time_ms),
        },
    ))
}

fn append_matches(
    relative_path: &Path,
    content: &str,
    query: &CompiledQuery,
    matches: &mut Vec<SearchLine>,
) {
    for (line_index, line) in content.lines().enumerate() {
        if query.is_match(line) {
            matches.push(SearchLine {
                path: relative_path.to_path_buf(),
                line: line_index + 1,
                content: line.to_string(),
            });
        }
    }
}

fn finalize_results(matches: Vec<SearchLine>, metadata: SearchMetadata) -> SearchResults {
    let lines = matches.iter().map(SearchLine::formatted).collect();
    SearchResults {
        lines,
        matches,
        metadata,
    }
}

fn slash_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_WORKSPACE_ID: AtomicUsize = AtomicUsize::new(0);

    struct TempWorkspace {
        root: PathBuf,
    }

    impl TempWorkspace {
        fn new() -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let id = NEXT_WORKSPACE_ID.fetch_add(1, Ordering::SeqCst);
            let root = std::env::temp_dir().join(format!(
                "roder-search-test-{}-{id}-{unique}",
                std::process::id(),
            ));
            fs::create_dir_all(&root).unwrap();
            Self { root }
        }

        fn write(&self, path: &str, content: impl AsRef<[u8]>) {
            let path = self.root.join(path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(path, content).unwrap();
        }
    }

    impl Drop for TempWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn literal_search_formats_lines() {
        let workspace = TempWorkspace::new();
        workspace.write("src/main.rs", "fn main() {}\nlet needle = true;\n");
        workspace.write("README.md", "nothing here\n");

        let results = search_workspace(&workspace.root, &SearchOptions::new("needle")).unwrap();

        assert_eq!(results.lines, vec!["src/main.rs:2:let needle = true;"]);
        assert_eq!(results.metadata.engine, SearchEngine::Indexed);
        assert_eq!(results.metadata.candidate_files, 1);
    }

    #[test]
    fn regex_search_verifies_final_matches() {
        let workspace = TempWorkspace::new();
        workspace.write("a.txt", "alpha\nalpza\nalpa\n");

        let options = SearchOptions::new("alp.a").regex(true);
        let results = search_workspace(&workspace.root, &options).unwrap();

        assert_eq!(results.lines, vec!["a.txt:1:alpha", "a.txt:2:alpza"]);
    }

    #[test]
    fn case_sensitivity_is_configurable() {
        let workspace = TempWorkspace::new();
        workspace.write("a.txt", "Alpha\nalpha\n");

        let sensitive = search_workspace(&workspace.root, &SearchOptions::new("alpha")).unwrap();
        let insensitive = search_workspace(
            &workspace.root,
            &SearchOptions::new("alpha").case_sensitive(false),
        )
        .unwrap();

        assert_eq!(sensitive.lines, vec!["a.txt:2:alpha"]);
        assert_eq!(insensitive.lines, vec!["a.txt:1:Alpha", "a.txt:2:alpha"]);
    }

    #[test]
    fn word_boundary_limits_literal_matches() {
        let workspace = TempWorkspace::new();
        workspace.write("a.txt", "cat\nscatter\ncat-like\n");

        let options = SearchOptions::new("cat").word_boundary(true);
        let results = search_workspace(&workspace.root, &options).unwrap();

        assert_eq!(results.lines, vec!["a.txt:1:cat", "a.txt:3:cat-like"]);
    }

    #[test]
    fn binary_files_are_skipped() {
        let workspace = TempWorkspace::new();
        workspace.write("binary.bin", b"\0needle");
        workspace.write("text.txt", "needle\n");

        let results = search_workspace(&workspace.root, &SearchOptions::new("needle")).unwrap();

        assert_eq!(results.lines, vec!["text.txt:1:needle"]);
    }

    #[test]
    fn ignored_dirs_are_skipped() {
        let workspace = TempWorkspace::new();
        workspace.write(".git/config", "needle\n");
        workspace.write("target/log.txt", "needle\n");
        workspace.write("src/lib.rs", "needle\n");

        let results = search_workspace(&workspace.root, &SearchOptions::new("needle")).unwrap();

        assert_eq!(results.lines, vec!["src/lib.rs:1:needle"]);
    }

    #[test]
    fn indexed_and_scan_modes_are_equivalent() {
        let workspace = TempWorkspace::new();
        workspace.write("a.txt", "alpha\nbeta\n");
        workspace.write("nested/b.txt", "alphabet\nnope\n");
        workspace.write("nested/c.txt", "ALPHA\n");
        workspace.write("nested/d.txt", "nothing\n");

        let scan = search_workspace(
            &workspace.root,
            &SearchOptions::new("alpha")
                .case_sensitive(false)
                .with_mode(SearchMode::Scan),
        )
        .unwrap();
        let indexed = search_workspace(
            &workspace.root,
            &SearchOptions::new("alpha")
                .case_sensitive(false)
                .with_mode(SearchMode::Indexed),
        )
        .unwrap();

        assert_eq!(indexed.lines, scan.lines);
        assert_eq!(indexed.metadata.engine, SearchEngine::Indexed);
        assert!(indexed.metadata.verified_files <= scan.metadata.verified_files);
    }

    #[test]
    fn indexed_mode_falls_back_when_query_has_no_trigrams() {
        let workspace = TempWorkspace::new();
        workspace.write("a.txt", "an\n");

        let results = search_workspace(
            &workspace.root,
            &SearchOptions::new("an").with_mode(SearchMode::Indexed),
        )
        .unwrap();

        assert_eq!(results.lines, vec!["a.txt:1:an"]);
        assert_eq!(results.metadata.engine, SearchEngine::Fallback);
    }

    #[test]
    fn respects_search_subpath() {
        let workspace = TempWorkspace::new();
        workspace.write("src/a.txt", "needle\n");
        workspace.write("tests/a.txt", "needle\n");

        let results = search_workspace(
            &workspace.root,
            &SearchOptions::new("needle").with_path("src"),
        )
        .unwrap();

        assert_eq!(results.lines, vec!["src/a.txt:1:needle"]);
    }

    #[test]
    fn cached_workspace_index_still_respects_later_query_paths() {
        let workspace = TempWorkspace::new();
        workspace.write("src/a.txt", "needle\n");
        workspace.write("tests/a.txt", "needle\n");
        let mut searcher = WorkspaceSearcher::new(&workspace.root);

        let src = searcher
            .search(&SearchOptions::new("needle").with_path("src"))
            .unwrap();
        let tests = searcher
            .search(&SearchOptions::new("needle").with_path("tests"))
            .unwrap();
        let all = searcher.search(&SearchOptions::new("needle")).unwrap();

        assert_eq!(src.lines, vec!["src/a.txt:1:needle"]);
        assert_eq!(tests.lines, vec!["tests/a.txt:1:needle"]);
        assert_eq!(
            all.lines,
            vec!["src/a.txt:1:needle", "tests/a.txt:1:needle"]
        );
    }
}
