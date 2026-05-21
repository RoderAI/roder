use std::path::{Path, PathBuf};

use roder_api::artifacts::{
    ArtifactGrepMatch, ArtifactGrepResult, ArtifactReadPage, ArtifactRetention, ContextArtifact,
    ContextArtifactKind,
};
use time::OffsetDateTime;

const DEFAULT_MAX_READ_BYTES: u64 = 2 * 1024 * 1024;
const DEFAULT_GREP_MATCH_LIMIT: usize = 200;

#[derive(Debug, Clone)]
pub struct ContextArtifactStore {
    root: PathBuf,
    max_read_bytes: u64,
    grep_match_limit: usize,
}

impl ContextArtifactStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            max_read_bytes: DEFAULT_MAX_READ_BYTES,
            grep_match_limit: DEFAULT_GREP_MATCH_LIMIT,
        }
    }

    pub fn with_max_read_bytes(mut self, max_read_bytes: u64) -> Self {
        self.max_read_bytes = max_read_bytes;
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn write(
        &self,
        thread_id: &str,
        turn_id: &str,
        kind: ContextArtifactKind,
        artifact_id: &str,
        source_tool_id: Option<&str>,
        label: &str,
        bytes: &[u8],
    ) -> anyhow::Result<ContextArtifact> {
        let relative_path = relative_artifact_path(turn_id, artifact_id, label);
        let path = self.root.join(&relative_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, bytes)?;
        let now = OffsetDateTime::now_utc();
        let artifact = ContextArtifact {
            id: artifact_id.to_string(),
            kind,
            thread_id: thread_id.to_string(),
            turn_id: turn_id.to_string(),
            source_tool_id: source_tool_id.map(str::to_string),
            relative_path,
            byte_size: bytes.len() as u64,
            line_count: count_lines(bytes),
            retention: ArtifactRetention {
                expires_at: None,
                pinned: false,
            },
            created_at: now,
            updated_at: now,
        };
        self.write_metadata(&artifact)?;
        Ok(artifact)
    }

    pub fn append(
        &self,
        artifact_id: &str,
        bytes: &[u8],
    ) -> anyhow::Result<(ContextArtifact, u64)> {
        let mut artifact = self.get(artifact_id)?;
        let path = self.data_path(&artifact);
        let mut file = std::fs::OpenOptions::new().append(true).open(&path)?;
        use std::io::Write;
        file.write_all(bytes)?;
        let appended = bytes.len() as u64;
        artifact.byte_size += appended;
        artifact.line_count += count_lines(bytes);
        artifact.updated_at = OffsetDateTime::now_utc();
        self.write_metadata(&artifact)?;
        Ok((artifact, appended))
    }

    pub fn get(&self, artifact_id: &str) -> anyhow::Result<ContextArtifact> {
        let path = self.metadata_path(artifact_id);
        let text = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&text)?)
    }

    pub fn get_for_thread(
        &self,
        thread_id: &str,
        artifact_id: &str,
    ) -> anyhow::Result<ContextArtifact> {
        let artifact = self.get(artifact_id)?;
        if artifact.thread_id != thread_id {
            anyhow::bail!(
                "artifact {artifact_id} belongs to thread {}, not {thread_id}",
                artifact.thread_id
            );
        }
        Ok(artifact)
    }

    pub fn list_for_thread(&self, thread_id: &str) -> anyhow::Result<Vec<ContextArtifact>> {
        let metadata_dir = self.root.join("metadata");
        if !metadata_dir.exists() {
            return Ok(Vec::new());
        }
        let mut artifacts = Vec::new();
        for entry in std::fs::read_dir(metadata_dir)? {
            let path = entry?.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let text = std::fs::read_to_string(&path)?;
            let artifact: ContextArtifact = serde_json::from_str(&text)?;
            if artifact.thread_id == thread_id {
                artifacts.push(artifact);
            }
        }
        artifacts.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(artifacts)
    }

    pub fn read_page(
        &self,
        thread_id: &str,
        artifact_id: &str,
        start_line: u64,
        limit: u64,
    ) -> anyhow::Result<ArtifactReadPage> {
        let artifact = self.get_for_thread(thread_id, artifact_id)?;
        let path = self.data_path(&artifact);
        let bytes = std::fs::read(&path)?;
        if bytes.len() as u64 > self.max_read_bytes {
            anyhow::bail!(
                "artifact {artifact_id} is {} bytes, over read limit {}",
                bytes.len(),
                self.max_read_bytes
            );
        }
        let text = String::from_utf8_lossy(&bytes);
        let lines: Vec<&str> = text.lines().collect();
        let total_lines = lines.len() as u64;
        let start = start_line.saturating_sub(1).min(total_lines) as usize;
        let end = start.saturating_add(limit as usize).min(lines.len());
        let page_lines = &lines[start..end];
        let next_start_line = if end < lines.len() {
            Some((end + 1) as u64)
        } else {
            None
        };
        Ok(ArtifactReadPage {
            artifact_id: artifact.id,
            start_line: start_line.max(1),
            line_count: page_lines.len() as u64,
            total_lines,
            text: page_lines.join("\n"),
            next_start_line,
        })
    }

    pub fn tail(
        &self,
        thread_id: &str,
        artifact_id: &str,
        lines: u64,
    ) -> anyhow::Result<String> {
        let artifact = self.get_for_thread(thread_id, artifact_id)?;
        let path = self.data_path(&artifact);
        let text = std::fs::read_to_string(&path).unwrap_or_else(|_| {
            String::from_utf8_lossy(&std::fs::read(&path).unwrap_or_default()).into_owned()
        });
        let all_lines: Vec<&str> = text.lines().collect();
        let start = all_lines.len().saturating_sub(lines as usize);
        Ok(all_lines[start..].join("\n"))
    }

    pub fn grep(
        &self,
        thread_id: &str,
        artifact_id: &str,
        pattern: &str,
    ) -> anyhow::Result<ArtifactGrepResult> {
        let artifact = self.get_for_thread(thread_id, artifact_id)?;
        let path = self.data_path(&artifact);
        let text = std::fs::read_to_string(&path).unwrap_or_else(|_| {
            String::from_utf8_lossy(&std::fs::read(&path).unwrap_or_default()).into_owned()
        });
        let mut matches = Vec::new();
        for (index, line) in text.lines().enumerate() {
            if line.contains(pattern) {
                matches.push(ArtifactGrepMatch {
                    line_number: (index + 1) as u64,
                    line: line.to_string(),
                });
            }
            if matches.len() >= self.grep_match_limit {
                break;
            }
        }
        let truncated = matches.len() >= self.grep_match_limit;
        Ok(ArtifactGrepResult {
            artifact_id: artifact.id,
            pattern: pattern.to_string(),
            matches,
            truncated,
        })
    }

    pub fn delete(&self, thread_id: &str, artifact_id: &str) -> anyhow::Result<bool> {
        let artifact = self.get_for_thread(thread_id, artifact_id)?;
        let mut deleted = false;
        let data_path = self.data_path(&artifact);
        if data_path.starts_with(&self.root) && data_path.exists() {
            std::fs::remove_file(data_path)?;
            deleted = true;
        }
        let metadata = self.metadata_path(artifact_id);
        if metadata.exists() {
            std::fs::remove_file(metadata)?;
            deleted = true;
        }
        Ok(deleted)
    }

    fn data_path(&self, artifact: &ContextArtifact) -> PathBuf {
        self.root.join(&artifact.relative_path)
    }

    fn write_metadata(&self, artifact: &ContextArtifact) -> anyhow::Result<()> {
        let path = self.metadata_path(&artifact.id);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(artifact)?)?;
        Ok(())
    }

    fn metadata_path(&self, artifact_id: &str) -> PathBuf {
        self.root.join("metadata").join(format!("{artifact_id}.json"))
    }
}

pub fn default_sessions_dir() -> anyhow::Result<PathBuf> {
    if let Ok(path) = std::env::var("RODER_SESSION_DIR") {
        return Ok(PathBuf::from(path));
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("could not resolve HOME for sessions"))?;
    Ok(home.join(".roder").join("sessions"))
}

/// Artifact payload root for one session: `{sessions_base}/{thread_id}/artifacts/`.
pub fn session_artifact_dir(sessions_base: impl AsRef<Path>, thread_id: &str) -> PathBuf {
    sessions_base
        .as_ref()
        .join(sanitize_path_segment(thread_id))
        .join("artifacts")
}

pub fn relative_artifact_path(turn_id: &str, artifact_id: &str, label: &str) -> String {
    format!(
        "{}/{}_{}.txt",
        sanitize_path_segment(turn_id),
        sanitize_path_segment(artifact_id),
        sanitize_path_segment(label)
    )
}

fn sanitize_path_segment(segment: &str) -> String {
    segment
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

fn count_lines(bytes: &[u8]) -> u64 {
    if bytes.is_empty() {
        return 0;
    }
    let mut count = 1u64;
    for byte in bytes {
        if *byte == b'\n' {
            count += 1;
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> ContextArtifactStore {
        ContextArtifactStore::new(
            std::env::temp_dir().join(format!("roder-context-artifacts-{}", uuid::Uuid::new_v4())),
        )
    }

    #[test]
    fn creates_reads_pages_greps_and_deletes_artifacts() {
        let store = temp_store();
        let body = (1..=50)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let artifact = store
            .write(
                "thread-a",
                "turn-b",
                ContextArtifactKind::ToolOutput,
                "call_123",
                Some("grep"),
                "stdout",
                body.as_bytes(),
            )
            .unwrap();

        assert!(artifact.relative_path.starts_with("turn-b/"));
        assert_eq!(artifact.line_count, 50);
        assert_eq!(store.list_for_thread("thread-a").unwrap().len(), 1);

        let page = store
            .read_page("thread-a", "call_123", 1, 5)
            .unwrap();
        assert_eq!(page.line_count, 5);
        assert_eq!(page.next_start_line, Some(6));

        let grep = store.grep("thread-a", "call_123", "line 42").unwrap();
        assert_eq!(grep.matches.len(), 1);

        let tail = store.tail("thread-a", "call_123", 3).unwrap();
        assert!(tail.contains("line 50"));

        assert!(store.delete("thread-a", "call_123").unwrap());
        assert!(store.list_for_thread("thread-a").unwrap().is_empty());
    }

    #[test]
    fn rejects_cross_thread_access() {
        let store = temp_store();
        store
            .write(
                "thread-a",
                "turn-b",
                ContextArtifactKind::ToolOutput,
                "call_1",
                None,
                "stdout",
                b"secret",
            )
            .unwrap();
        assert!(store.get_for_thread("thread-b", "call_1").is_err());
    }
}
