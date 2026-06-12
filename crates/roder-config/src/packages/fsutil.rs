//! Filesystem helpers shared by the package fetchers: staged copies with an
//! atomic swap into the store, and deterministic content hashing.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::Context;
use sha2::{Digest, Sha256};

static UNIQUE_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Process-unique suffix for staging directories.
pub(crate) fn unique_suffix() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    let count = UNIQUE_COUNTER.fetch_add(1, Ordering::SeqCst);
    format!("{}-{nanos}-{count}", std::process::id())
}

/// Hidden staging sibling next to `final_path` so the final `rename` never
/// crosses filesystems.
pub(crate) fn staging_sibling(final_path: &Path, label: &str) -> PathBuf {
    let name = format!(".{label}-{}", unique_suffix());
    match final_path.parent() {
        Some(parent) => parent.join(name),
        None => PathBuf::from(name),
    }
}

/// Recursive copy preserving directory structure. Symlinks are recreated on
/// unix (npm `node_modules/.bin` entries) and skipped elsewhere.
pub(crate) fn copy_dir_recursive(from: &Path, to: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(to).with_context(|| format!("create directory {}", to.display()))?;
    let entries =
        fs::read_dir(from).with_context(|| format!("read directory {}", from.display()))?;
    for entry in entries {
        let entry = entry.with_context(|| format!("read directory {}", from.display()))?;
        let file_type = entry
            .file_type()
            .with_context(|| format!("stat {}", entry.path().display()))?;
        let source = entry.path();
        let dest = to.join(entry.file_name());
        if file_type.is_symlink() {
            #[cfg(unix)]
            {
                let target = fs::read_link(&source)
                    .with_context(|| format!("read symlink {}", source.display()))?;
                std::os::unix::fs::symlink(&target, &dest).with_context(|| {
                    format!(
                        "recreate symlink {} -> {}",
                        dest.display(),
                        target.display()
                    )
                })?;
            }
        } else if file_type.is_dir() {
            copy_dir_recursive(&source, &dest)?;
        } else {
            fs::copy(&source, &dest)
                .with_context(|| format!("copy {} -> {}", source.display(), dest.display()))?;
        }
    }
    Ok(())
}

/// Swaps a fully staged directory into its final location. An existing tree
/// is parked aside first and restored if the swap fails, so consumers never
/// observe a half-written store entry.
pub(crate) fn swap_dir_into_place(staged: &Path, final_path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = final_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create store directory {}", parent.display()))?;
    }
    let backup = staging_sibling(final_path, "replaced");
    let had_existing = final_path.exists();
    if had_existing {
        fs::rename(final_path, &backup)
            .with_context(|| format!("move previous install aside at {}", final_path.display()))?;
    }
    match fs::rename(staged, final_path) {
        Ok(()) => {
            if had_existing {
                let _ = fs::remove_dir_all(&backup);
            }
            Ok(())
        }
        Err(err) => {
            if had_existing {
                let _ = fs::rename(&backup, final_path);
            }
            Err(err).with_context(|| {
                format!(
                    "move staged package {} into {}",
                    staged.display(),
                    final_path.display()
                )
            })
        }
    }
}

/// Deterministic sha2-256 over sorted relative file paths plus file bytes,
/// skipping `.git` and `node_modules` so VCS metadata and refreshed deps do
/// not churn the hash.
pub fn content_hash(root: &Path) -> anyhow::Result<String> {
    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    files.sort();
    let mut hasher = Sha256::new();
    for rel in &files {
        hasher.update(rel.as_bytes());
        hasher.update([0u8]);
        let path = root.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        let bytes = fs::read(&path).with_context(|| format!("read {}", path.display()))?;
        hasher.update(&bytes);
        hasher.update([0u8]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn collect_files(root: &Path, dir: &Path, files: &mut Vec<String>) -> anyhow::Result<()> {
    let entries = fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))?;
    for entry in entries {
        let entry = entry.with_context(|| format!("read {}", dir.display()))?;
        let file_type = entry
            .file_type()
            .with_context(|| format!("stat {}", entry.path().display()))?;
        let name = entry.file_name();
        if file_type.is_dir() {
            if name == ".git" || name == "node_modules" {
                continue;
            }
            collect_files(root, &entry.path(), files)?;
        } else if file_type.is_file() {
            let path = entry.path();
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            files.push(rel);
        }
        // Symlinks are skipped: hashing follows regular files only.
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("roder-pkg-fsutil-{name}-{}", unique_suffix()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn content_hash_is_deterministic_and_skips_git_and_node_modules() {
        let root = tempdir("hash");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/a.txt"), "alpha").unwrap();
        fs::write(root.join("b.txt"), "beta").unwrap();
        let first = content_hash(&root).unwrap();

        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join(".git/config"), "noise").unwrap();
        fs::create_dir_all(root.join("node_modules/dep")).unwrap();
        fs::write(root.join("node_modules/dep/index.js"), "noise").unwrap();
        assert_eq!(content_hash(&root).unwrap(), first);

        fs::write(root.join("b.txt"), "changed").unwrap();
        assert_ne!(content_hash(&root).unwrap(), first);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn swap_replaces_existing_tree_atomically() {
        let base = tempdir("swap");
        let staged = base.join("staged");
        let target = base.join("final");
        fs::create_dir_all(&staged).unwrap();
        fs::write(staged.join("new.txt"), "new").unwrap();
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("old.txt"), "old").unwrap();

        swap_dir_into_place(&staged, &target).unwrap();
        assert!(target.join("new.txt").exists());
        assert!(!target.join("old.txt").exists());
        assert!(!staged.exists());
        let _ = fs::remove_dir_all(base);
    }
}
