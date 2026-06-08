use std::path::{Path, PathBuf};

pub use roder_api::artifacts::CreateArtifactRequest;
use roder_api::artifacts::{
    ArtifactGrepPage, ArtifactReadPage, ArtifactTailPage, ContextArtifact, ContextArtifactAccess,
    ContextArtifactId, ContextArtifactStore as SharedContextArtifactStore,
};
use roder_api::events::{ThreadId, TurnId};
use time::OffsetDateTime;

const DEFAULT_PAGE_LINES: usize = 200;
const MAX_PAGE_LINES: usize = 200;

#[derive(Debug, Clone)]
pub struct ContextArtifactStore {
    root: PathBuf,
    layout: ArtifactStorageLayout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArtifactStorageLayout {
    LegacyRoot,
    ThreadScoped,
}

impl ContextArtifactStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            layout: ArtifactStorageLayout::LegacyRoot,
        }
    }

    pub fn new_thread_scoped(thread_root: impl Into<PathBuf>) -> Self {
        Self {
            root: thread_root.into(),
            layout: ArtifactStorageLayout::ThreadScoped,
        }
    }

    pub fn shared_legacy(root: impl Into<PathBuf>) -> SharedContextArtifactStore {
        SharedContextArtifactStore::new(std::sync::Arc::new(Self::new(root)))
    }

    pub fn shared_thread_scoped(thread_root: impl Into<PathBuf>) -> SharedContextArtifactStore {
        SharedContextArtifactStore::new(std::sync::Arc::new(Self::new_thread_scoped(thread_root)))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn create(&self, request: CreateArtifactRequest<'_>) -> anyhow::Result<ContextArtifact> {
        let id = format!("artifact-{}", uuid::Uuid::new_v4());
        let dir = self.turn_dir(request.thread_id, request.turn_id);
        std::fs::create_dir_all(&dir)?;
        let data_path = dir.join(format!("{id}.txt"));
        std::fs::write(&data_path, request.bytes)?;
        let artifact = ContextArtifact {
            id: id.clone(),
            kind: request.kind,
            thread_id: request.thread_id.clone(),
            turn_id: request.turn_id.clone(),
            byte_count: request.bytes.len() as u64,
            line_count: line_count_lossy(request.bytes) as u64,
            source_tool_id: request.source_tool_id.map(ToOwned::to_owned),
            label: request.label.map(ToOwned::to_owned),
            store_path: data_path.display().to_string(),
            retention_expires_at: None,
            created_at: OffsetDateTime::now_utc(),
            roder_owned: true,
        };
        self.write_metadata(&artifact)?;
        Ok(artifact)
    }

    pub fn append(
        &self,
        thread_id: &ThreadId,
        artifact_id: &ContextArtifactId,
        bytes: &[u8],
    ) -> anyhow::Result<ContextArtifact> {
        let mut artifact = self.get_scoped(thread_id, artifact_id)?;
        let path = PathBuf::from(&artifact.store_path);
        ensure_under_root(&self.root, &path)?;
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().append(true).open(&path)?;
        file.write_all(bytes)?;
        let all_bytes = std::fs::read(&path)?;
        artifact.byte_count = all_bytes.len() as u64;
        artifact.line_count = line_count_lossy(&all_bytes) as u64;
        self.write_metadata(&artifact)?;
        Ok(artifact)
    }

    pub fn get_scoped(
        &self,
        thread_id: &ThreadId,
        artifact_id: &ContextArtifactId,
    ) -> anyhow::Result<ContextArtifact> {
        let artifact = self.get(artifact_id)?;
        if &artifact.thread_id != thread_id {
            anyhow::bail!("artifact {artifact_id} does not belong to thread {thread_id}");
        }
        Ok(artifact)
    }

    pub fn get(&self, artifact_id: &ContextArtifactId) -> anyhow::Result<ContextArtifact> {
        for metadata in self.metadata_paths()? {
            if metadata.file_stem().and_then(|stem| stem.to_str()) != Some(artifact_id.as_str()) {
                continue;
            }
            let text = std::fs::read_to_string(&metadata)?;
            return Ok(serde_json::from_str(&text)?);
        }
        anyhow::bail!("unknown artifact {artifact_id}")
    }

    fn read_bytes_scoped(
        &self,
        thread_id: &ThreadId,
        artifact_id: &ContextArtifactId,
    ) -> anyhow::Result<(ContextArtifact, Vec<u8>)> {
        let artifact = self.get_scoped(thread_id, artifact_id)?;
        let path = PathBuf::from(&artifact.store_path);
        ensure_under_root(&self.root, &path)?;
        Ok((artifact, std::fs::read(path)?))
    }

    fn metadata_paths(&self) -> anyhow::Result<Vec<PathBuf>> {
        let mut out = Vec::new();
        if !self.root.exists() {
            return Ok(out);
        }
        collect_metadata_paths(&self.root, &mut out)?;
        Ok(out)
    }

    fn list_thread(&self, thread_id: &ThreadId) -> anyhow::Result<Vec<ContextArtifact>> {
        let mut artifacts = Vec::new();
        for metadata in self.metadata_paths()? {
            let text = std::fs::read_to_string(metadata)?;
            let artifact: ContextArtifact = serde_json::from_str(&text)?;
            if &artifact.thread_id == thread_id {
                artifacts.push(artifact);
            }
        }
        artifacts.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then(left.id.cmp(&right.id))
        });
        Ok(artifacts)
    }

    fn delete_scoped(
        &self,
        thread_id: &ThreadId,
        artifact_id: &ContextArtifactId,
    ) -> anyhow::Result<bool> {
        let artifact = self.get_scoped(thread_id, artifact_id)?;
        if !artifact.roder_owned {
            anyhow::bail!(
                "refusing to delete non-Roder-owned artifact {}",
                artifact.id
            );
        }
        let data_path = PathBuf::from(&artifact.store_path);
        ensure_under_root(&self.root, &data_path)?;
        let metadata_path = self.metadata_path(&artifact);
        let mut deleted = false;
        if data_path.exists() {
            std::fs::remove_file(data_path)?;
            deleted = true;
        }
        if metadata_path.exists() {
            std::fs::remove_file(metadata_path)?;
            deleted = true;
        }
        Ok(deleted)
    }

    fn write_metadata(&self, artifact: &ContextArtifact) -> anyhow::Result<()> {
        let path = self.metadata_path(artifact);
        std::fs::write(path, serde_json::to_string_pretty(artifact)?)?;
        Ok(())
    }

    fn metadata_path(&self, artifact: &ContextArtifact) -> PathBuf {
        self.turn_dir(&artifact.thread_id, &artifact.turn_id)
            .join(format!("{}.json", artifact.id))
    }

    fn turn_dir(&self, thread_id: &ThreadId, turn_id: &TurnId) -> PathBuf {
        let thread_dir = self.root.join(safe_component(thread_id));
        match self.layout {
            ArtifactStorageLayout::LegacyRoot => thread_dir.join(safe_component(turn_id)),
            ArtifactStorageLayout::ThreadScoped => {
                thread_dir.join("artifacts").join(safe_component(turn_id))
            }
        }
    }
}

impl ContextArtifactAccess for ContextArtifactStore {
    fn create_artifact(
        &self,
        request: CreateArtifactRequest<'_>,
    ) -> anyhow::Result<ContextArtifact> {
        self.create(request)
    }

    fn append_artifact(
        &self,
        thread_id: &ThreadId,
        artifact_id: &ContextArtifactId,
        bytes: &[u8],
    ) -> anyhow::Result<ContextArtifact> {
        self.append(thread_id, artifact_id, bytes)
    }

    fn list_artifacts(&self, thread_id: &ThreadId) -> anyhow::Result<Vec<ContextArtifact>> {
        self.list_thread(thread_id)
    }

    fn read_artifact(
        &self,
        thread_id: &ThreadId,
        artifact_id: &ContextArtifactId,
        start_line: usize,
        limit: usize,
    ) -> anyhow::Result<ArtifactReadPage> {
        let (artifact, bytes) = self.read_bytes_scoped(thread_id, artifact_id)?;
        let lines = numbered_lines(&bytes);
        let start_line = start_line.max(1);
        let limit = clamp_limit(Some(limit));
        let page = page_lines(&lines, start_line - 1, limit);
        Ok(ArtifactReadPage {
            artifact: artifact.descriptor(),
            text: page.text,
            start_line,
            limit,
            shown: page.shown,
            total_lines: page.total,
            next_start_line: page.next_offset.map(|offset| offset + 1),
            truncated: page.next_offset.is_some(),
        })
    }

    fn grep_artifact(
        &self,
        thread_id: &ThreadId,
        artifact_id: &ContextArtifactId,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> anyhow::Result<ArtifactGrepPage> {
        if query.is_empty() {
            anyhow::bail!("query is required");
        }
        let (artifact, bytes) = self.read_bytes_scoped(thread_id, artifact_id)?;
        let matches = String::from_utf8_lossy(&bytes)
            .lines()
            .enumerate()
            .filter(|(_, line)| line.contains(query))
            .map(|(index, line)| format!("{}: {}", index + 1, line))
            .collect::<Vec<_>>();
        let limit = clamp_limit(Some(limit));
        let page = page_lines(&matches, offset, limit);
        Ok(ArtifactGrepPage {
            artifact: artifact.descriptor(),
            query: query.to_string(),
            text: page.text,
            offset,
            limit,
            shown: page.shown,
            total_matches: page.total,
            next_offset: page.next_offset,
            truncated: page.next_offset.is_some(),
        })
    }

    fn tail_artifact(
        &self,
        thread_id: &ThreadId,
        artifact_id: &ContextArtifactId,
        lines: usize,
    ) -> anyhow::Result<ArtifactTailPage> {
        let (artifact, bytes) = self.read_bytes_scoped(thread_id, artifact_id)?;
        let all_lines = numbered_lines(&bytes);
        let lines = clamp_limit(Some(lines));
        let total = all_lines.len();
        let start = total.saturating_sub(lines);
        let page = page_lines(&all_lines, start, lines);
        Ok(ArtifactTailPage {
            artifact: artifact.descriptor(),
            text: page.text,
            start_line: start + 1,
            lines,
            shown: page.shown,
            total_lines: page.total,
            truncated: start > 0,
        })
    }

    fn delete_artifact(
        &self,
        thread_id: &ThreadId,
        artifact_id: &ContextArtifactId,
    ) -> anyhow::Result<bool> {
        self.delete_scoped(thread_id, artifact_id)
    }
}

pub fn default_context_artifact_dir() -> PathBuf {
    std::env::var_os("RODER_DATA_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".roder")))
        .unwrap_or_else(|| PathBuf::from(".roder"))
        .join("context-artifacts")
}

fn safe_component(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn collect_metadata_paths(dir: &Path, out: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_metadata_paths(&path, out)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("json")
            && path
                .file_stem()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("artifact-"))
        {
            out.push(path);
        }
    }
    Ok(())
}

fn line_count_lossy(bytes: &[u8]) -> usize {
    let text = String::from_utf8_lossy(bytes);
    if text.is_empty() {
        0
    } else {
        text.lines().count()
    }
}

fn numbered_lines(bytes: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .enumerate()
        .map(|(index, line)| format!("{:>5}: {}", index + 1, line))
        .collect()
}

#[derive(Debug, Clone)]
struct LinePage {
    text: String,
    shown: usize,
    total: usize,
    next_offset: Option<usize>,
}

fn clamp_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(DEFAULT_PAGE_LINES).clamp(1, MAX_PAGE_LINES)
}

fn page_lines(lines: &[String], offset: usize, limit: usize) -> LinePage {
    let total = lines.len();
    let offset = offset.min(total);
    let end = offset.saturating_add(limit).min(total);
    let next_offset = (end < total).then_some(end);
    let mut text = lines[offset..end].join("\n");
    if let Some(next) = next_offset {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&format!(
            "[showing lines {}-{} of {total}; next_offset={next}]",
            offset + 1,
            end
        ));
    }
    LinePage {
        text,
        shown: end.saturating_sub(offset),
        total,
        next_offset,
    }
}

fn ensure_under_root(root: &Path, path: &Path) -> anyhow::Result<()> {
    let root = root.canonicalize()?;
    let path = path.canonicalize()?;
    if !path.starts_with(root) {
        anyhow::bail!("artifact path escapes artifact root");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::artifacts::ContextArtifactKind;

    #[test]
    fn artifact_store_writes_thread_scoped_reads_greps_tails_and_deletes_by_thread() {
        let root =
            std::env::temp_dir().join(format!("roder-context-artifacts-{}", uuid::Uuid::new_v4()));
        let store = ContextArtifactStore::new_thread_scoped(&root);
        let artifact = store
            .create(CreateArtifactRequest {
                kind: ContextArtifactKind::ToolOutput,
                thread_id: &"thread-a".to_string(),
                turn_id: &"turn-a".to_string(),
                source_tool_id: Some("call-a"),
                label: Some("stdout"),
                bytes: b"alpha\nneedle\nomega\n",
            })
            .unwrap();

        assert!(
            artifact
                .store_path
                .starts_with(root.to_string_lossy().as_ref())
        );
        let store_path = Path::new(&artifact.store_path);
        assert!(store_path.starts_with(root.join("thread-a").join("artifacts").join("turn-a")));
        assert!(
            store_path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("artifact-"))
        );
        assert_eq!(artifact.line_count, 3);
        assert_eq!(
            store.list_artifacts(&"thread-a".to_string()).unwrap().len(),
            1
        );

        let read = store
            .read_artifact(&"thread-a".to_string(), &artifact.id, 2, 1)
            .unwrap();
        assert!(read.text.contains("2: needle"));
        assert_eq!(read.next_start_line, Some(3));

        let grep = store
            .grep_artifact(&"thread-a".to_string(), &artifact.id, "needle", 0, 10)
            .unwrap();
        assert_eq!(grep.text, "2: needle");

        let tail = store
            .tail_artifact(&"thread-a".to_string(), &artifact.id, 2)
            .unwrap();
        assert!(tail.text.contains("2: needle"));
        assert!(tail.text.contains("3: omega"));

        let wrong_thread = store
            .read_artifact(&"thread-b".to_string(), &artifact.id, 1, 1)
            .unwrap_err()
            .to_string();
        assert!(wrong_thread.contains("does not belong to thread"));

        assert!(
            store
                .delete_artifact(&"thread-a".to_string(), &artifact.id)
                .unwrap()
        );
        assert!(
            store
                .list_artifacts(&"thread-a".to_string())
                .unwrap()
                .is_empty()
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn artifact_store_appends_and_recounts_lossy_utf8() {
        let root =
            std::env::temp_dir().join(format!("roder-context-artifacts-{}", uuid::Uuid::new_v4()));
        let store = ContextArtifactStore::new(&root);
        let artifact = store
            .create(CreateArtifactRequest {
                kind: ContextArtifactKind::CommandStderr,
                thread_id: &"thread-a".to_string(),
                turn_id: &"turn-a".to_string(),
                source_tool_id: None,
                label: Some("stderr"),
                bytes: b"one\n",
            })
            .unwrap();

        let artifact = store
            .append(&"thread-a".to_string(), &artifact.id, b"\xfftwo\n")
            .unwrap();

        assert_eq!(artifact.line_count, 2);
        let read = store
            .read_artifact(&"thread-a".to_string(), &artifact.id, 1, 10)
            .unwrap();
        assert!(read.text.contains("two"));
        let _ = std::fs::remove_dir_all(root);
    }
}
