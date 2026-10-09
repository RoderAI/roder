//! What a live run measured: the commit, the crate's files as they stood,
//! the corpus and the model, taken at the start of a run and again at its
//! end.
//!
//! Numbers taken against a tree that changed underneath them describe
//! nothing. A shared checkout edited in the middle of a benchmark has
//! already cost QuickE2E one (`fixtures/run.mjs`, 2026-09-21), so a run
//! reads its pin twice and a difference between the two reads is a mid-run
//! edit. A baseline saved from such a run would pin the wrong thing, so it
//! is refused.
//!
//! Hashes are SHA-256 over each file's path, length and bytes in path order,
//! cut to 16 hex digits: they say that something changed, not what.

use std::path::Path;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The corpus, relative to the crate: the tasks and the pages they open.
const CORPUS: [&str; 2] = ["tests/fixtures/evals/tasks.json", "tests/fixtures/pages"];

/// What the code under test is made of: the library, its tests and fixtures.
const WORKTREE: [&str; 3] = ["Cargo.toml", "src", "tests"];

/// The saved baseline sits in the tree it describes, so a save must not move
/// the worktree hash of the next run.
pub(super) const BASELINE_FILE: &str = "live-baseline.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Pin {
    /// `git rev-parse HEAD`, or `unknown` outside a checkout.
    pub(crate) commit: String,
    /// The crate's sources, tests and fixtures, committed or not.
    pub(crate) worktree: String,
    /// `tasks.json` and the fixture pages.
    pub(crate) corpus: String,
    /// The decision model the run asked for.
    pub(crate) model: String,
    /// The switches that change what a run measures: variants, text model,
    /// fallback, gate and cookie-banner refusal.
    pub(crate) setup: String,
}

impl Pin {
    /// Read the pin from the crate's own directory.
    pub(crate) fn capture(model: &str, setup: &str) -> anyhow::Result<Self> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        Ok(Self {
            commit: commit(root),
            worktree: hash_paths(root, &WORKTREE, &[BASELINE_FILE])?,
            corpus: hash_paths(root, &CORPUS, &[])?,
            model: model.into(),
            setup: setup.into(),
        })
    }

    /// What changed between this read and a later one of the same run.
    /// Empty means nothing moved underneath the numbers.
    pub(crate) fn drift(&self, later: &Self) -> Vec<String> {
        let mut changed = Vec::new();
        for (name, was, now) in [
            ("commit", &self.commit, &later.commit),
            ("worktree", &self.worktree, &later.worktree),
            ("corpus", &self.corpus, &later.corpus),
            ("model", &self.model, &later.model),
            ("setup", &self.setup, &later.setup),
        ] {
            if was != now {
                changed.push(format!("{name} {was} -> {now}"));
            }
        }
        changed
    }
}

fn commit(root: &Path) -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|sha| sha.trim().to_string())
        .filter(|sha| !sha.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

/// Hash the files at `paths` (each a file or a directory, relative to
/// `root`), leaving out any file named in `skip` and any `target` or `.git`
/// directory. A path that does not exist hashes as itself, absent, so a
/// removed fixture changes the hash like an edited one.
pub(crate) fn hash_paths(root: &Path, paths: &[&str], skip: &[&str]) -> anyhow::Result<String> {
    let mut files = Vec::new();
    for path in paths {
        collect(root, &root.join(path), skip, &mut files)?;
    }
    let mut hash = Sha256::new();
    for (relative, bytes) in files {
        hash.update(relative.as_bytes());
        hash.update([0]);
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(&bytes);
    }
    Ok(hash
        .finalize()
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn collect(
    root: &Path,
    path: &Path,
    skip: &[&str],
    files: &mut Vec<(String, Vec<u8>)>,
) -> anyhow::Result<()> {
    let relative = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    if path.is_dir() {
        let mut entries = std::fs::read_dir(path)
            .with_context(|| format!("read {}", path.display()))?
            .collect::<Result<Vec<_>, _>>()
            .with_context(|| format!("read {}", path.display()))?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            if matches!(name.as_str(), "target" | ".git") {
                continue;
            }
            collect(root, &entry.path(), skip, files)?;
        }
    } else if path.is_file() {
        let name = path.file_name().map(|name| name.to_string_lossy());
        if !name.is_some_and(|name| skip.contains(&name.as_ref())) {
            let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
            files.push((relative, bytes));
        }
    } else {
        files.push((format!("{relative} (absent)"), Vec::new()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    /// A scratch tree of our own, removed on drop.
    struct Tree(PathBuf);

    impl Tree {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "jev-pin-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::SeqCst)
            ));
            std::fs::create_dir_all(dir.join("pages")).unwrap();
            Self(dir)
        }

        fn write(&self, name: &str, text: &str) {
            let path = self.0.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }

        fn hash(&self) -> String {
            hash_paths(&self.0, &["tasks.json", "pages"], &["live-baseline.json"]).unwrap()
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    #[test]
    fn an_edit_a_new_file_and_a_removed_file_each_change_the_hash() {
        let tree = Tree::new();
        tree.write("tasks.json", "[]");
        tree.write("pages/a.html", "<p>a</p>");
        let first = tree.hash();
        assert_eq!(first.len(), 16);
        assert_eq!(tree.hash(), first, "the same tree hashes the same");

        tree.write("pages/a.html", "<p>b</p>");
        let edited = tree.hash();
        assert_ne!(edited, first);

        tree.write("pages/b.html", "");
        let added = tree.hash();
        assert_ne!(added, edited);

        std::fs::remove_file(tree.0.join("pages/b.html")).unwrap();
        assert_eq!(tree.hash(), edited, "removing the new file undoes it");
        std::fs::remove_file(tree.0.join("pages/a.html")).unwrap();
        assert_ne!(tree.hash(), edited);
    }

    #[test]
    fn the_same_bytes_under_another_name_hash_differently() {
        let one = Tree::new();
        one.write("tasks.json", "[]");
        one.write("pages/a.html", "x");
        let two = Tree::new();
        two.write("tasks.json", "[]");
        two.write("pages/b.html", "x");
        assert_ne!(one.hash(), two.hash());
    }

    #[test]
    fn the_baseline_file_and_build_output_are_not_part_of_the_tree() {
        let tree = Tree::new();
        tree.write("tasks.json", "[]");
        let before = tree.hash();
        tree.write("pages/live-baseline.json", "{}");
        tree.write("pages/target/out.bin", "built");
        assert_eq!(tree.hash(), before);
    }

    #[test]
    fn a_pin_reads_this_crate_and_its_corpus() {
        let pin = Pin::capture("jev-latest", "task values").unwrap();
        assert_eq!(pin.worktree.len(), 16, "{pin:?}");
        assert_eq!(pin.corpus.len(), 16, "{pin:?}");
        assert!(
            pin.commit == "unknown" || pin.commit.len() == 40,
            "{}",
            pin.commit
        );
        // Nothing between two reads is nothing to flag.
        assert_eq!(
            pin.drift(&Pin::capture("jev-latest", "task values").unwrap()),
            Vec::<String>::new()
        );
    }

    #[test]
    fn drift_names_each_part_that_moved() {
        let pin = Pin {
            commit: "aaa".into(),
            worktree: "1111".into(),
            corpus: "2222".into(),
            model: "jev-latest".into(),
            setup: "task values".into(),
        };
        assert!(pin.drift(&pin.clone()).is_empty());
        let later = Pin {
            worktree: "3333".into(),
            corpus: "4444".into(),
            ..pin.clone()
        };
        assert_eq!(
            pin.drift(&later),
            ["worktree 1111 -> 3333", "corpus 2222 -> 4444"]
        );
    }
}
