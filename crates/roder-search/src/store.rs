use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::INDEX_VERSION;
use crate::index::SearchIndex;

pub const STORE_MANIFEST_FILE: &str = "manifest.json";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndexStoreMetadata {
    pub index_version: String,
    pub workspace_key: String,
    pub workspace_root: PathBuf,
    pub document_count: usize,
    pub index_bytes: u64,
    pub build_time_ms: u128,
    pub created_at_ms: u128,
}

impl IndexStoreMetadata {
    pub fn from_index(index: &SearchIndex) -> Self {
        Self {
            index_version: INDEX_VERSION.to_string(),
            workspace_key: workspace_key(index.root()),
            workspace_root: index.root().to_path_buf(),
            document_count: index.stats().document_count,
            index_bytes: index.stats().index_bytes,
            build_time_ms: index.stats().build_time_ms,
            created_at_ms: now_ms(),
        }
    }
}

pub fn workspace_key(root: impl AsRef<Path>) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    root.as_ref().to_string_lossy().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

pub fn default_store_dir(home: impl AsRef<Path>, workspace_root: impl AsRef<Path>) -> PathBuf {
    home.as_ref()
        .join(".roder")
        .join("indexes")
        .join(workspace_key(workspace_root))
}

pub fn manifest_path(store_dir: impl AsRef<Path>) -> PathBuf {
    store_dir.as_ref().join(STORE_MANIFEST_FILE)
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_dir_is_workspace_scoped() {
        let one = default_store_dir("/tmp/home", "/tmp/work/a");
        let two = default_store_dir("/tmp/home", "/tmp/work/b");
        assert_ne!(one, two);
        assert_eq!(manifest_path(&one), one.join(STORE_MANIFEST_FILE));
    }
}
