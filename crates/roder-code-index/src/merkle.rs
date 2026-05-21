use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use ignore::{DirEntry, WalkBuilder};
use roder_api::code_index::{
    CodeIndexNodeKind, ContentHash, MerkleHash, PathHash, WorkspaceMerkleNode, WorkspaceMerkleTree,
    WorkspaceSimilarityHash,
};

use crate::hex_sha256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MerkleBuildOptions {
    pub scopes: Vec<PathBuf>,
}

impl Default for MerkleBuildOptions {
    fn default() -> Self {
        Self {
            scopes: vec![PathBuf::from(".")],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileManifestEntry {
    pub path: PathBuf,
    pub path_hash: PathHash,
    pub content_hash: ContentHash,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MerkleBuild {
    pub tree: WorkspaceMerkleTree,
    pub files: Vec<FileManifestEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MerkleDiff {
    pub changed_files: Vec<PathBuf>,
    pub deleted_files: Vec<PathBuf>,
    pub unchanged_files: Vec<PathBuf>,
}

pub fn build_workspace_merkle(root: impl AsRef<Path>) -> anyhow::Result<MerkleBuild> {
    build_workspace_merkle_with_options(root, &MerkleBuildOptions::default())
}

pub fn build_workspace_merkle_with_options(
    root: impl AsRef<Path>,
    options: &MerkleBuildOptions,
) -> anyhow::Result<MerkleBuild> {
    let workspace_root = fs::canonicalize(root.as_ref())
        .with_context(|| format!("canonicalize workspace {}", root.as_ref().display()))?;
    if !workspace_root.is_dir() {
        bail!(
            "workspace root is not a directory: {}",
            workspace_root.display()
        );
    }

    let scopes = canonical_scopes(&workspace_root, &options.scopes)?;
    let mut files = Vec::new();
    let mut directory_children: BTreeMap<PathBuf, BTreeSet<MerkleHash>> = BTreeMap::new();

    let mut walk = WalkBuilder::new(&workspace_root);
    walk.standard_filters(true).hidden(false);
    for result in walk.build() {
        let entry = result?;
        if should_skip(&workspace_root, &entry) {
            continue;
        }
        let path = entry.path();
        if path == workspace_root {
            continue;
        }
        if !in_scope(path, &scopes) {
            continue;
        }
        if !path.is_file() {
            continue;
        }

        let rel = path
            .strip_prefix(&workspace_root)
            .with_context(|| format!("strip workspace root from {}", path.display()))?
            .to_path_buf();
        let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
        let content_hash = hex_sha256(&bytes);
        let path_hash = hash_path(&rel);
        let file_hash = hash_node("file", &rel, &[content_hash.as_str()]);
        files.push(FileManifestEntry {
            path: rel.clone(),
            path_hash: path_hash.clone(),
            content_hash,
            size: bytes.len() as u64,
        });
        for ancestor in rel.ancestors().skip(1) {
            directory_children
                .entry(ancestor.to_path_buf())
                .or_default()
                .insert(file_hash.clone());
        }
    }

    files.sort_by(|a, b| a.path.cmp(&b.path));

    let mut nodes = Vec::new();
    for file in &files {
        nodes.push(WorkspaceMerkleNode {
            path: file.path.clone(),
            path_hash: file.path_hash.clone(),
            content_hash: hash_node("file", &file.path, &[file.content_hash.as_str()]),
            kind: CodeIndexNodeKind::File,
            children: Vec::new(),
        });
    }

    let mut directory_hashes = BTreeMap::<PathBuf, MerkleHash>::new();
    let mut directories = directory_children.keys().cloned().collect::<Vec<_>>();
    directories.sort_by(|a, b| {
        b.components()
            .count()
            .cmp(&a.components().count())
            .then(a.cmp(b))
    });
    for directory in directories {
        let mut children = directory_children
            .get(&directory)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect::<Vec<_>>();
        for (child_dir, child_hash) in &directory_hashes {
            if child_dir.parent() == Some(directory.as_path()) {
                children.push(child_hash.clone());
            }
        }
        children.sort();
        let child_refs = children.iter().map(String::as_str).collect::<Vec<_>>();
        let content_hash = hash_node("dir", &directory, &child_refs);
        directory_hashes.insert(directory.clone(), content_hash.clone());
        nodes.push(WorkspaceMerkleNode {
            path: directory.clone(),
            path_hash: hash_path(&directory),
            content_hash,
            kind: CodeIndexNodeKind::Directory,
            children,
        });
    }

    let root_hash = directory_hashes
        .get(Path::new(""))
        .cloned()
        .unwrap_or_else(|| hash_node("dir", Path::new(""), &[]));
    nodes.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then_with(|| node_kind_rank(&a.kind).cmp(&node_kind_rank(&b.kind)))
    });

    Ok(MerkleBuild {
        tree: WorkspaceMerkleTree {
            workspace_root,
            root_hash: root_hash.clone(),
            similarity_hash: WorkspaceSimilarityHash {
                algorithm: "roder-simhash-v1".to_string(),
                value: similarity_hash(&files),
            },
            nodes,
        },
        files,
    })
}

pub fn diff_file_manifests(
    previous: &[FileManifestEntry],
    current: &[FileManifestEntry],
) -> MerkleDiff {
    let previous_by_path = previous
        .iter()
        .map(|entry| (entry.path.clone(), entry.content_hash.clone()))
        .collect::<BTreeMap<_, _>>();
    let current_by_path = current
        .iter()
        .map(|entry| (entry.path.clone(), entry.content_hash.clone()))
        .collect::<BTreeMap<_, _>>();

    let mut changed_files = Vec::new();
    let mut deleted_files = Vec::new();
    let mut unchanged_files = Vec::new();

    for (path, content_hash) in &current_by_path {
        match previous_by_path.get(path) {
            Some(previous_hash) if previous_hash == content_hash => {
                unchanged_files.push(path.clone())
            }
            _ => changed_files.push(path.clone()),
        }
    }
    for path in previous_by_path.keys() {
        if !current_by_path.contains_key(path) {
            deleted_files.push(path.clone());
        }
    }

    MerkleDiff {
        changed_files,
        deleted_files,
        unchanged_files,
    }
}

pub fn hash_path(path: &Path) -> PathHash {
    hex_sha256(normalize_path(path))
}

fn canonical_scopes(workspace_root: &Path, scopes: &[PathBuf]) -> anyhow::Result<Vec<PathBuf>> {
    if scopes.is_empty() {
        return Ok(vec![workspace_root.to_path_buf()]);
    }
    let mut out = Vec::new();
    for scope in scopes {
        let joined = if scope.is_absolute() {
            scope.clone()
        } else {
            workspace_root.join(scope)
        };
        let canonical = fs::canonicalize(&joined)
            .with_context(|| format!("canonicalize index scope {}", joined.display()))?;
        if !canonical.starts_with(workspace_root) {
            bail!("index scope escapes workspace: {}", canonical.display());
        }
        out.push(canonical);
    }
    Ok(out)
}

fn in_scope(path: &Path, scopes: &[PathBuf]) -> bool {
    scopes.iter().any(|scope| path.starts_with(scope))
}

fn should_skip(root: &Path, entry: &DirEntry) -> bool {
    let path = entry.path();
    let Ok(rel) = path.strip_prefix(root) else {
        return true;
    };
    rel.components().any(|component| {
        let value = component.as_os_str().to_string_lossy();
        matches!(
            value.as_ref(),
            ".git" | ".roder" | "target" | "node_modules"
        )
    })
}

fn hash_node(kind: &str, path: &Path, parts: &[&str]) -> MerkleHash {
    let mut material = String::new();
    material.push_str(kind);
    material.push('\0');
    material.push_str(&normalize_path(path));
    for part in parts {
        material.push('\0');
        material.push_str(part);
    }
    hex_sha256(material)
}

fn normalize_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn similarity_hash(files: &[FileManifestEntry]) -> String {
    let mut material = String::new();
    for file in files {
        material.push_str(&normalize_path(&file.path));
        material.push('\0');
        material.push_str(&file.content_hash[..16.min(file.content_hash.len())]);
        material.push('\n');
    }
    hex_sha256(material)[..16].to_string()
}

fn node_kind_rank(kind: &CodeIndexNodeKind) -> u8 {
    match kind {
        CodeIndexNodeKind::Directory => 0,
        CodeIndexNodeKind::File => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merkle_respects_ignore_rules_and_path_scope() {
        let root = tempdir("merkle_respects_ignore_rules_and_path_scope");
        write(root.join("src/lib.rs"), "pub fn allowed() {}\n");
        write(root.join("src/private.rs"), "pub fn private() {}\n");
        write(root.join(".roder/state.json"), "{}\n");
        write(root.join("target/debug/output"), "ignored\n");
        write(root.join(".git/HEAD"), "ignored\n");

        let build = build_workspace_merkle_with_options(
            &root,
            &MerkleBuildOptions {
                scopes: vec![PathBuf::from("src/lib.rs")],
            },
        )
        .unwrap();

        let paths = build
            .files
            .iter()
            .map(|file| file.path.clone())
            .collect::<Vec<_>>();
        assert_eq!(paths, vec![PathBuf::from("src/lib.rs")]);
        assert!(
            build
                .tree
                .nodes
                .iter()
                .any(|node| node.path == PathBuf::from("src/lib.rs"))
        );
    }

    #[test]
    fn merkle_detects_changed_and_deleted_files() {
        let root = tempdir("merkle_detects_changed_and_deleted_files");
        write(root.join("src/a.rs"), "pub fn a() {}\n");
        write(root.join("src/b.rs"), "pub fn b() {}\n");
        let first = build_workspace_merkle(&root).unwrap();

        write(root.join("src/a.rs"), "pub fn a_changed() {}\n");
        fs::remove_file(root.join("src/b.rs")).unwrap();
        let second = build_workspace_merkle(&root).unwrap();
        let diff = diff_file_manifests(&first.files, &second.files);

        assert_eq!(diff.changed_files, vec![PathBuf::from("src/a.rs")]);
        assert_eq!(diff.deleted_files, vec![PathBuf::from("src/b.rs")]);
        assert_ne!(first.tree.root_hash, second.tree.root_hash);
    }

    fn write(path: PathBuf, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn tempdir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "roder-code-index-{name}-{}-{}",
            std::process::id(),
            time::OffsetDateTime::now_utc().unix_timestamp_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }
}
