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
fn gitignored_paths_are_skipped_in_scan_and_index() {
    let workspace = TempWorkspace::new();
    workspace.write(".gitignore", ".claude/\nignored.log\n");
    workspace.write(".claude/worktrees/agent/app.rs", "needle ignored\n");
    workspace.write("ignored.log", "needle ignored\n");
    workspace.write("src/lib.rs", "needle kept\n");

    let scan = search_workspace(
        &workspace.root,
        &SearchOptions::new("needle").with_mode(SearchMode::Scan),
    )
    .unwrap();
    let indexed = search_workspace(
        &workspace.root,
        &SearchOptions::new("needle").with_mode(SearchMode::Indexed),
    )
    .unwrap();

    assert_eq!(scan.lines, vec!["src/lib.rs:1:needle kept"]);
    assert_eq!(indexed.lines, scan.lines);
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

#[test]
fn scoped_indexed_search_warms_only_requested_file() {
    let workspace = TempWorkspace::new();
    workspace.write("src/types/roder.ts", "export type toolSubject = string;\n");
    workspace.write(
        "node_modules/big-package/index.js",
        "toolSubject in dependency\n",
    );
    let mut searcher = WorkspaceSearcher::new(&workspace.root);

    let results = searcher
        .search(&SearchOptions::new("toolSubject").with_path("src/types/roder.ts"))
        .unwrap();

    assert_eq!(
        results.lines,
        vec!["src/types/roder.ts:1:export type toolSubject = string;"]
    );
    let index = searcher.index().expect("search should warm an index");
    assert_eq!(index.scope(), workspace.root.join("src/types/roder.ts"));
    assert_eq!(index.stats().document_count, 1);
}
