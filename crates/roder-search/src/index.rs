use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Component, Path, PathBuf};
use std::time::{Instant, UNIX_EPOCH};

use ignore::WalkBuilder;

use crate::postings::{FileId, Trigram, intersect_postings, trigrams};
use crate::query::CompiledQuery;
use crate::{INDEX_VERSION, SearchError, SearchOptions};

#[derive(Clone, Debug)]
pub struct Document {
    pub id: FileId,
    pub path: PathBuf,
    pub content_hash: u64,
    pub size: u64,
    pub modified_ms: Option<u128>,
    pub language_hint: Option<String>,
}

#[derive(Clone, Debug)]
pub struct IndexStats {
    pub index_version: String,
    pub document_count: usize,
    pub index_bytes: u64,
    pub build_time_ms: u128,
}

#[derive(Clone, Debug)]
pub struct SearchIndex {
    root: PathBuf,
    scope: PathBuf,
    documents: Vec<Document>,
    postings: BTreeMap<Trigram, BTreeSet<FileId>>,
    stats: IndexStats,
}

impl SearchIndex {
    pub fn build(root: impl AsRef<Path>, options: &SearchOptions) -> Result<Self, SearchError> {
        let start = Instant::now();
        let root = root.as_ref().to_path_buf();
        let scope = scoped_path(&root, &options.path)?;
        let mut files = collect_text_files(&root, &scope, options.max_file_size)?;
        files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));

        let mut documents = Vec::with_capacity(files.len());
        let mut postings: BTreeMap<Trigram, BTreeSet<FileId>> = BTreeMap::new();

        for (id, file) in files.into_iter().enumerate() {
            for trigram in trigrams(&file.content, false) {
                postings.entry(trigram).or_default().insert(id);
            }

            documents.push(Document {
                id,
                path: file.relative_path,
                content_hash: file.content_hash,
                size: file.size,
                modified_ms: file.modified_ms,
                language_hint: file.language_hint,
            });
        }

        let index_bytes = estimate_index_bytes(&documents, &postings);
        let stats = IndexStats {
            index_version: INDEX_VERSION.to_string(),
            document_count: documents.len(),
            index_bytes,
            build_time_ms: start.elapsed().as_millis(),
        };

        Ok(Self {
            root,
            scope,
            documents,
            postings,
            stats,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn scope(&self) -> &Path {
        &self.scope
    }

    pub fn documents(&self) -> &[Document] {
        &self.documents
    }

    pub fn stats(&self) -> &IndexStats {
        &self.stats
    }

    pub(crate) fn candidate_file_ids(&self, query: &CompiledQuery) -> Option<BTreeSet<FileId>> {
        let mut postings = Vec::new();
        for trigram in query.required_trigrams() {
            match self.postings.get(trigram) {
                Some(posting) => postings.push(posting),
                None => return Some(BTreeSet::new()),
            }
        }

        if postings.is_empty() {
            None
        } else {
            Some(intersect_postings(postings.into_iter()))
        }
    }

    pub(crate) fn document_path(&self, id: FileId) -> Option<PathBuf> {
        self.documents
            .get(id)
            .map(|document| self.root.join(&document.path))
    }

    pub(crate) fn from_persisted(
        root: PathBuf,
        scope: PathBuf,
        documents: Vec<Document>,
        postings: BTreeMap<Trigram, BTreeSet<FileId>>,
        stats: IndexStats,
    ) -> Self {
        Self {
            root,
            scope,
            documents,
            postings,
            stats,
        }
    }

    pub(crate) fn postings(&self) -> &BTreeMap<Trigram, BTreeSet<FileId>> {
        &self.postings
    }

    pub(crate) fn document_is_stale(&self, id: FileId) -> bool {
        let Some(document) = self.documents.get(id) else {
            return true;
        };
        let path = self.root.join(&document.path);
        let Ok(metadata) = fs::metadata(&path) else {
            return true;
        };

        document.size != metadata.len() || document.modified_ms != modified_ms(&metadata)
    }

    pub fn has_stale_documents(&self) -> bool {
        self.documents
            .iter()
            .any(|document| self.document_is_stale(document.id))
    }
}

pub(crate) struct TextFile {
    pub(crate) relative_path: PathBuf,
    pub(crate) content: String,
    pub(crate) content_hash: u64,
    pub(crate) size: u64,
    pub(crate) modified_ms: Option<u128>,
    pub(crate) language_hint: Option<String>,
}

pub(crate) fn scoped_path(root: &Path, subpath: &Path) -> Result<PathBuf, SearchError> {
    // Absolute paths address the filesystem directly. Access is gated upstream by
    // the workspace's path scope (`resolve_existing`), so here we just normalize
    // away any `.`/`..` noise and let full paths through.
    if subpath.is_absolute() {
        return normalize_absolute(subpath);
    }

    let mut normalized = root.to_path_buf();
    for component in subpath.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => normalized.push(part),
            Component::ParentDir => {
                if !normalized.pop() || !normalized.starts_with(root) {
                    return Err(SearchError::InvalidPath(
                        "search path must stay inside the workspace".to_string(),
                    ));
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(SearchError::InvalidPath(
                    "search path must stay inside the workspace".to_string(),
                ));
            }
        }
    }

    Ok(normalized)
}

fn normalize_absolute(path: &Path) -> Result<PathBuf, SearchError> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::Normal(part) => normalized.push(part),
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(SearchError::InvalidPath(
                        "search path escapes the filesystem root".to_string(),
                    ));
                }
            }
        }
    }
    Ok(normalized)
}

/// The base directory used to compute workspace-relative document paths. For
/// scopes inside the workspace this is the workspace `root`, so matches display
/// as workspace-relative. For absolute scopes outside the workspace, the scope
/// itself is the base so paths display relative to the directory being searched.
pub(crate) fn search_base<'a>(root: &'a Path, scope: &'a Path) -> &'a Path {
    if scope.starts_with(root) { root } else { scope }
}

pub(crate) fn collect_text_files(
    root: &Path,
    scope: &Path,
    max_file_size: u64,
) -> Result<Vec<TextFile>, SearchError> {
    let base = search_base(root, scope).to_path_buf();
    let mut files = Vec::new();
    let mut walk = WalkBuilder::new(scope);
    walk.standard_filters(true)
        .hidden(false)
        .require_git(false)
        .filter_entry({
            let base = base.clone();
            move |entry| !ignored_path(&base, entry.path())
        })
        .sort_by_file_path(|left, right| left.cmp(right));

    for entry in walk.build() {
        let entry = entry.map_err(ignore_error)?;
        let path = entry.path();
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => return Err(SearchError::Io(err)),
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_file()
            && let Some(file) = read_text_file(&base, path, &metadata, max_file_size)?
        {
            files.push(file);
        }
    }
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(files)
}

fn read_text_file(
    root: &Path,
    path: &Path,
    metadata: &fs::Metadata,
    max_file_size: u64,
) -> Result<Option<TextFile>, SearchError> {
    if metadata.len() > max_file_size || obvious_binary(path) {
        return Ok(None);
    }

    let bytes = fs::read(path)?;
    if bytes.contains(&0) {
        return Ok(None);
    }
    let Ok(content) = String::from_utf8(bytes) else {
        return Ok(None);
    };

    let relative_path = path
        .strip_prefix(root)
        .unwrap_or(path)
        .components()
        .collect::<PathBuf>();
    let content_hash = hash_content(&content);

    Ok(Some(TextFile {
        relative_path,
        content,
        content_hash,
        size: metadata.len(),
        modified_ms: modified_ms(metadata),
        language_hint: language_hint(path),
    }))
}

fn ignored_path(root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return true;
    };
    relative.components().any(|component| {
        let value = component.as_os_str().to_string_lossy();
        matches!(
            value.as_ref(),
            ".cache"
                | ".git"
                | ".gradle"
                | ".next"
                | ".nuxt"
                | ".parcel-cache"
                | ".roder"
                | ".svelte-kit"
                | ".turbo"
                | ".vite"
                | ".yarn"
                | "DerivedData"
                | "Pods"
                | "build"
                | "coverage"
                | "dist"
                | "node_modules"
                | "out"
                | "target"
        )
    })
}

fn ignore_error(err: ignore::Error) -> SearchError {
    SearchError::Io(std::io::Error::other(err))
}

fn obvious_binary(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .is_some_and(|extension| {
            matches!(
                extension.as_str(),
                "7z" | "a"
                    | "bin"
                    | "bmp"
                    | "class"
                    | "db"
                    | "dylib"
                    | "exe"
                    | "gif"
                    | "gz"
                    | "ico"
                    | "jar"
                    | "jpeg"
                    | "jpg"
                    | "o"
                    | "pdf"
                    | "png"
                    | "sqlite"
                    | "tar"
                    | "ttf"
                    | "wasm"
                    | "webp"
                    | "woff"
                    | "woff2"
                    | "zip"
            )
        })
}

fn modified_ms(metadata: &fs::Metadata) -> Option<u128> {
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis())
}

fn language_hint(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_string)
}

fn hash_content(content: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    content.hash(&mut hasher);
    hasher.finish()
}

fn estimate_index_bytes(
    documents: &[Document],
    postings: &BTreeMap<Trigram, BTreeSet<FileId>>,
) -> u64 {
    let document_bytes = documents
        .iter()
        .map(|document| document.path.as_os_str().len() as u64 + 48)
        .sum::<u64>();
    let postings_bytes = postings
        .values()
        .map(|ids| 3 + (ids.len() as u64 * std::mem::size_of::<FileId>() as u64))
        .sum::<u64>();
    document_bytes + postings_bytes
}
