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
    IncrementalStoreStats, IndexStoreMetadata, LoadedSearchIndex, STORE_LOOKUP_FILE,
    STORE_MANIFEST_FILE, STORE_POSTINGS_DIR, default_store_dir, load_persistent_index, lookup_path,
    manifest_path, postings_dir, rebuild_persistent_index, save_incremental_index, workspace_key,
};

pub const INDEX_VERSION: &str = "roder-search-v2";
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
    Store(String),
}

impl fmt::Display for SearchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(formatter, "{err}"),
            Self::InvalidPath(message) | Self::InvalidQuery(message) => {
                formatter.write_str(message)
            }
            Self::Store(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for SearchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::InvalidPath(_) | Self::InvalidQuery(_) | Self::Store(_) => None,
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

    pub fn with_index(root: impl AsRef<Path>, index: SearchIndex) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
            index: Some(index),
        }
    }

    pub fn warm(&mut self, options: &SearchOptions) -> Result<&SearchIndex, SearchError> {
        self.index = Some(SearchIndex::build(&self.root, options)?);
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
                let requested_scope = scoped_path(&self.root, &options.path)?;
                // The persistent index is rooted at the workspace; absolute paths
                // that point outside it can't be served from the index, so scan.
                if !requested_scope.starts_with(&self.root) {
                    return scan_search(
                        &self.root,
                        options,
                        &query,
                        start,
                        SearchEngine::Scan,
                        None,
                    );
                }
                // Scopes inside skip-listed directories (node_modules, dist,
                // ...) are never in the workspace index; scan them directly.
                if index::scope_is_ignored(&self.root, &requested_scope) {
                    return scan_search(
                        &self.root,
                        options,
                        &query,
                        start,
                        SearchEngine::Scan,
                        None,
                    );
                }
                let should_warm = match self.index.as_ref() {
                    Some(index) => !requested_scope.starts_with(index.scope()),
                    None => true,
                };
                if should_warm {
                    self.warm(options)?;
                }
                let index = self.index.as_ref().expect("index is initialized");
                let stats = index.stats().clone();
                // A scope the index holds no documents for (gitignored at build
                // time, or created since) can't be answered from the index;
                // scanning keeps explicitly targeted paths searchable.
                if requested_scope != self.root && !index.has_documents_under(&requested_scope) {
                    return scan_search(
                        &self.root,
                        options,
                        &query,
                        start,
                        SearchEngine::Fallback,
                        Some(&stats),
                    );
                }
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
#[path = "lib_tests.rs"]
mod tests;
