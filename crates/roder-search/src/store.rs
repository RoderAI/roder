use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{collections::BTreeMap, collections::BTreeSet, fs};

use crate::INDEX_VERSION;
use crate::index::{Document, IndexStats, SearchIndex, scoped_path};
use crate::postings::{FileId, Trigram};
use crate::{SearchError, SearchOptions};

pub const STORE_MANIFEST_FILE: &str = "manifest.json";
pub const STORE_LOOKUP_FILE: &str = "lookup.json";
pub const STORE_POSTINGS_DIR: &str = "postings";

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IncrementalStoreStats {
    pub metadata: IndexStoreMetadata,
    pub changed_documents: usize,
    pub reused_documents: usize,
}

#[derive(Clone, Debug)]
pub struct LoadedSearchIndex {
    pub index: SearchIndex,
    pub metadata: IndexStoreMetadata,
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredManifest {
    index_version: String,
    workspace_key: String,
    workspace_root: PathBuf,
    document_count: usize,
    index_bytes: u64,
    build_time_ms: u128,
    created_at_ms: u128,
    lookup_file: String,
    postings_dir: String,
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredDocument {
    id: FileId,
    path: PathBuf,
    content_hash: u64,
    size: u64,
    modified_ms: Option<u128>,
    language_hint: Option<String>,
    postings_file: String,
}

#[derive(Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredLookup {
    documents: Vec<StoredDocument>,
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredDocumentPostings {
    trigrams: Vec<String>,
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

pub fn lookup_path(store_dir: impl AsRef<Path>) -> PathBuf {
    store_dir.as_ref().join(STORE_LOOKUP_FILE)
}

pub fn postings_dir(store_dir: impl AsRef<Path>) -> PathBuf {
    store_dir.as_ref().join(STORE_POSTINGS_DIR)
}

pub fn rebuild_persistent_index(
    store_dir: impl AsRef<Path>,
    workspace_root: impl AsRef<Path>,
    options: &SearchOptions,
) -> Result<IncrementalStoreStats, SearchError> {
    let workspace_root = workspace_root.as_ref();
    let index = SearchIndex::build(workspace_root, options)?;
    save_incremental_index(store_dir, &index)
}

pub fn save_incremental_index(
    store_dir: impl AsRef<Path>,
    index: &SearchIndex,
) -> Result<IncrementalStoreStats, SearchError> {
    let store_dir = store_dir.as_ref();
    fs::create_dir_all(postings_dir(store_dir))?;
    let previous = load_lookup(store_dir).unwrap_or_default();
    let previous_by_path = previous
        .documents
        .into_iter()
        .map(|document| (document.path.clone(), document))
        .collect::<BTreeMap<_, _>>();
    let document_trigrams = document_trigrams(index);
    let mut changed_documents = 0usize;
    let mut reused_documents = 0usize;
    let mut stored_documents = Vec::with_capacity(index.documents().len());

    for document in index.documents() {
        let postings_file = posting_file_name(document.id);
        let stored = StoredDocument {
            id: document.id,
            path: document.path.clone(),
            content_hash: document.content_hash,
            size: document.size,
            modified_ms: document.modified_ms,
            language_hint: document.language_hint.clone(),
            postings_file: postings_file.clone(),
        };
        let unchanged = previous_by_path
            .get(&document.path)
            .is_some_and(|previous| document_unchanged(previous, &stored));
        let path = postings_dir(store_dir).join(&postings_file);
        if unchanged && path.exists() {
            reused_documents += 1;
        } else {
            changed_documents += 1;
            let trigrams = document_trigrams
                .get(&document.id)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(Trigram::key)
                .collect::<Vec<_>>();
            let postings = StoredDocumentPostings { trigrams };
            write_json(&path, &postings)?;
        }
        stored_documents.push(stored);
    }

    remove_stale_postings(store_dir, &stored_documents)?;
    write_json(
        lookup_path(store_dir),
        &StoredLookup {
            documents: stored_documents,
        },
    )?;
    let metadata = IndexStoreMetadata::from_index(index);
    write_json(
        manifest_path(store_dir),
        &StoredManifest {
            index_version: metadata.index_version.clone(),
            workspace_key: metadata.workspace_key.clone(),
            workspace_root: metadata.workspace_root.clone(),
            document_count: metadata.document_count,
            index_bytes: metadata.index_bytes,
            build_time_ms: metadata.build_time_ms,
            created_at_ms: metadata.created_at_ms,
            lookup_file: STORE_LOOKUP_FILE.to_string(),
            postings_dir: STORE_POSTINGS_DIR.to_string(),
        },
    )?;

    Ok(IncrementalStoreStats {
        metadata,
        changed_documents,
        reused_documents,
    })
}

pub fn load_persistent_index(
    store_dir: impl AsRef<Path>,
    workspace_root: impl AsRef<Path>,
    options: &SearchOptions,
) -> Result<Option<LoadedSearchIndex>, SearchError> {
    let store_dir = store_dir.as_ref();
    let manifest_path = manifest_path(store_dir);
    if !manifest_path.exists() {
        return Ok(None);
    }
    let manifest: StoredManifest = read_json(manifest_path)?;
    let root = workspace_root.as_ref().to_path_buf();
    if manifest.index_version != INDEX_VERSION || manifest.workspace_key != workspace_key(&root) {
        return Ok(None);
    }
    let lookup = load_lookup(store_dir)?;
    let mut documents = Vec::with_capacity(lookup.documents.len());
    let mut postings = BTreeMap::<Trigram, BTreeSet<FileId>>::new();
    for stored in lookup.documents {
        for trigram in load_document_postings(store_dir, &stored.postings_file)? {
            postings.entry(trigram).or_default().insert(stored.id);
        }
        documents.push(Document {
            id: stored.id,
            path: stored.path,
            content_hash: stored.content_hash,
            size: stored.size,
            modified_ms: stored.modified_ms,
            language_hint: stored.language_hint,
        });
    }
    documents.sort_by_key(|document| document.id);
    let scope = scoped_path(&root, &options.path)?;
    let stats = IndexStats {
        index_version: manifest.index_version.clone(),
        document_count: manifest.document_count,
        index_bytes: manifest.index_bytes,
        build_time_ms: manifest.build_time_ms,
    };
    Ok(Some(LoadedSearchIndex {
        index: SearchIndex::from_persisted(root, scope, documents, postings, stats),
        metadata: IndexStoreMetadata {
            index_version: manifest.index_version,
            workspace_key: manifest.workspace_key,
            workspace_root: manifest.workspace_root,
            document_count: manifest.document_count,
            index_bytes: manifest.index_bytes,
            build_time_ms: manifest.build_time_ms,
            created_at_ms: manifest.created_at_ms,
        },
    }))
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

fn document_trigrams(index: &SearchIndex) -> BTreeMap<FileId, BTreeSet<Trigram>> {
    let mut by_document = BTreeMap::<FileId, BTreeSet<Trigram>>::new();
    for (trigram, file_ids) in index.postings() {
        for id in file_ids {
            by_document.entry(*id).or_default().insert(*trigram);
        }
    }
    by_document
}

fn document_unchanged(previous: &StoredDocument, current: &StoredDocument) -> bool {
    previous.content_hash == current.content_hash
        && previous.size == current.size
        && previous.language_hint == current.language_hint
}

fn posting_file_name(id: FileId) -> String {
    format!("{id:016x}.json")
}

fn load_lookup(store_dir: &Path) -> Result<StoredLookup, SearchError> {
    read_json(lookup_path(store_dir))
}

fn load_document_postings(store_dir: &Path, file_name: &str) -> Result<Vec<Trigram>, SearchError> {
    let postings: StoredDocumentPostings = read_json(postings_dir(store_dir).join(file_name))?;
    Ok(postings
        .trigrams
        .iter()
        .filter_map(|key| Trigram::from_key(key))
        .collect())
}

fn remove_stale_postings(
    store_dir: &Path,
    documents: &[StoredDocument],
) -> Result<(), SearchError> {
    let active = documents
        .iter()
        .map(|document| document.postings_file.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let dir = postings_dir(store_dir);
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !active.contains(name) {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

fn read_json<T: serde::de::DeserializeOwned>(path: impl AsRef<Path>) -> Result<T, SearchError> {
    let text = fs::read_to_string(path)?;
    serde_json::from_str(&text).map_err(|err| SearchError::Store(err.to_string()))
}

fn write_json<T: serde::Serialize>(path: impl AsRef<Path>, value: &T) -> Result<(), SearchError> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text =
        serde_json::to_string_pretty(value).map_err(|err| SearchError::Store(err.to_string()))?;
    fs::write(path, text)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_STORE_ID: AtomicUsize = AtomicUsize::new(0);
    static NEXT_WORKSPACE_ID: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn store_dir_is_workspace_scoped() {
        let one = default_store_dir("/tmp/home", "/tmp/work/a");
        let two = default_store_dir("/tmp/home", "/tmp/work/b");
        assert_ne!(one, two);
        assert_eq!(manifest_path(&one), one.join(STORE_MANIFEST_FILE));
    }

    #[test]
    fn store_persists_lookup_and_postings_separately() {
        let workspace = TempWorkspace::new();
        workspace.write("src/lib.rs", "pub fn needle() {}\n");
        workspace.write("src/main.rs", "fn main() {}\n");
        let store_dir = unique_store_dir();
        let options = SearchOptions::new("needle");
        let index = SearchIndex::build(&workspace.root, &options).unwrap();

        let stats = save_incremental_index(&store_dir, &index).unwrap();
        let loaded = load_persistent_index(&store_dir, &workspace.root, &options)
            .unwrap()
            .unwrap();
        let mut searcher = crate::WorkspaceSearcher::with_index(&workspace.root, loaded.index);
        let results = searcher.search(&options).unwrap();

        assert_eq!(stats.changed_documents, 2);
        assert!(manifest_path(&store_dir).exists());
        assert!(lookup_path(&store_dir).exists());
        assert!(postings_dir(&store_dir).is_dir());
        assert_eq!(loaded.metadata.index_version, INDEX_VERSION);
        assert_eq!(results.lines, vec!["src/lib.rs:1:pub fn needle() {}"]);
    }

    #[test]
    fn incremental_store_rewrites_only_changed_document_postings() {
        let workspace = TempWorkspace::new();
        workspace.write("a.txt", "alpha needle\n");
        workspace.write("b.txt", "beta needle\n");
        let store_dir = unique_store_dir();
        let options = SearchOptions::new("needle");

        let first = rebuild_persistent_index(&store_dir, &workspace.root, &options).unwrap();
        workspace.write("b.txt", "beta needle changed\n");
        let second = rebuild_persistent_index(&store_dir, &workspace.root, &options).unwrap();

        assert_eq!(first.changed_documents, 2);
        assert_eq!(second.changed_documents, 1);
        assert_eq!(second.reused_documents, 1);
    }

    struct TempWorkspace {
        root: PathBuf,
    }

    impl TempWorkspace {
        fn new() -> Self {
            let id = NEXT_WORKSPACE_ID.fetch_add(1, Ordering::SeqCst);
            let root = std::env::temp_dir().join(format!(
                "roder-search-store-workspace-{}-{id}-{}",
                std::process::id(),
                now_ms()
            ));
            fs::create_dir_all(&root).unwrap();
            Self { root }
        }

        fn write(&self, path: &str, content: &str) {
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

    fn unique_store_dir() -> PathBuf {
        let id = NEXT_STORE_ID.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "roder-search-store-{}-{id}-{}",
            std::process::id(),
            now_ms()
        ))
    }
}
