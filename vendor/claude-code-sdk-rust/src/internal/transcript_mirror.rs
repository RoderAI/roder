use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::error::Result;
use crate::session_store::{
    file_path_to_session_key, SessionStoreEntry, SessionStoreHandle, SessionSummaryEntry,
};
use crate::types::{ClaudeAgentOptions, Message, SessionStoreFlushMode};

const MAX_PENDING_ENTRIES: usize = 500;
const MAX_PENDING_BYTES: usize = 1 << 20;

#[derive(Debug, Clone)]
struct PendingMirrorFrame {
    file_path: PathBuf,
    entries: Vec<SessionStoreEntry>,
}

#[derive(Debug, Clone)]
pub struct TranscriptMirrorBatcher {
    store: SessionStoreHandle,
    projects_dir: PathBuf,
    pending: Vec<PendingMirrorFrame>,
    pending_entries: usize,
    pending_bytes: usize,
    flush_mode: SessionStoreFlushMode,
    mirror_errors: Vec<Message>,
}

impl TranscriptMirrorBatcher {
    pub fn from_options(options: &ClaudeAgentOptions) -> Option<Self> {
        let store = options.session_store.clone()?;
        Some(Self {
            store,
            projects_dir: projects_dir_for_options(options),
            pending: Vec::new(),
            pending_entries: 0,
            pending_bytes: 0,
            flush_mode: options.session_store_flush,
            mirror_errors: Vec::new(),
        })
    }

    pub async fn enqueue_value(&mut self, value: &serde_json::Value) -> Result<Vec<Message>> {
        let Some(file_path) = value.get("filePath").and_then(|v| v.as_str()) else {
            return Ok(Vec::new());
        };
        let entries = value
            .get("entries")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.as_object().cloned())
            .collect::<Vec<_>>();
        if entries.is_empty() {
            return Ok(Vec::new());
        }

        let bytes = serde_json::to_vec(&entries)?.len();
        self.pending_entries += entries.len();
        self.pending_bytes += bytes;
        self.pending.push(PendingMirrorFrame {
            file_path: PathBuf::from(file_path),
            entries,
        });

        if self.flush_mode == SessionStoreFlushMode::Eager
            || self.pending_entries > MAX_PENDING_ENTRIES
            || self.pending_bytes > MAX_PENDING_BYTES
        {
            return self.flush().await;
        }

        Ok(Vec::new())
    }

    pub async fn flush(&mut self) -> Result<Vec<Message>> {
        if self.pending.is_empty() {
            return Ok(std::mem::take(&mut self.mirror_errors));
        }

        let frames = std::mem::take(&mut self.pending);
        self.pending_entries = 0;
        self.pending_bytes = 0;

        let mut by_path = BTreeMap::<PathBuf, Vec<SessionStoreEntry>>::new();
        for frame in frames {
            by_path
                .entry(frame.file_path)
                .or_default()
                .extend(frame.entries);
        }

        for (file_path, entries) in by_path {
            let Some(key) = file_path_to_session_key(&file_path, &self.projects_dir) else {
                continue;
            };
            if let Err(error) = self.store.append(key.clone(), entries).await {
                self.mirror_errors.push(mirror_error_message(
                    Some(file_path.to_string_lossy().to_string()),
                    Some(key),
                    error.to_string(),
                ));
            }
        }

        Ok(std::mem::take(&mut self.mirror_errors))
    }

    pub fn pending_bytes(&self) -> usize {
        self.pending_bytes
    }

    pub fn pending_entries(&self) -> usize {
        self.pending_entries
    }
}

fn projects_dir_for_options(options: &ClaudeAgentOptions) -> PathBuf {
    options
        .env
        .get("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".claude")))
        .unwrap_or_else(|| PathBuf::from(".claude"))
        .join("projects")
}

pub fn mirror_error_message(
    file_path: Option<String>,
    key: Option<crate::session_store::SessionKey>,
    error: impl Into<String>,
) -> Message {
    let mut data = serde_json::Map::new();
    let error = error.into();
    data.insert(
        "error".to_string(),
        serde_json::Value::String(error.clone()),
    );
    if let Some(file_path) = file_path {
        data.insert("filePath".to_string(), serde_json::Value::String(file_path));
    }
    let key = key.map(|key| {
        let mut value = serde_json::Map::new();
        value.insert(
            "project_key".to_string(),
            serde_json::json!(key.project_key),
        );
        value.insert("session_id".to_string(), serde_json::json!(key.session_id));
        if let Some(subpath) = key.subpath {
            value.insert("subpath".to_string(), serde_json::json!(subpath));
        }
        value
    });
    if let Some(key) = &key {
        data.insert("key".to_string(), serde_json::Value::Object(key.clone()));
    }
    Message::MirrorErrorMsg(crate::types::MirrorErrorMessage { key, error, data })
}

#[allow(dead_code)]
fn _keep_session_summary_entry_public(_: Option<SessionSummaryEntry>) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_store::{InMemorySessionStore, SessionKey, SessionStore};
    use async_trait::async_trait;

    struct FailingStore;

    #[async_trait]
    impl SessionStore for FailingStore {
        async fn append(&self, _key: SessionKey, _entries: Vec<SessionStoreEntry>) -> Result<()> {
            Err(crate::error::ClaudeSDKError::Session(
                "store failed".to_string(),
            ))
        }

        async fn load(&self, _key: SessionKey) -> Result<Option<Vec<SessionStoreEntry>>> {
            Ok(None)
        }
    }

    fn mirror_frame(projects_dir: &std::path::Path, uuid: &str) -> serde_json::Value {
        serde_json::json!({
            "type": "transcript_mirror",
            "filePath": projects_dir.join("proj/session-1.jsonl"),
            "entries": [{
                "type": "user",
                "uuid": uuid,
                "message": {"content": format!("prompt {uuid}")}
            }]
        })
    }

    #[tokio::test]
    async fn batcher_coalesces_and_flushes_by_session_key() {
        let store = InMemorySessionStore::new();
        let temp =
            std::env::temp_dir().join(format!("claude-rust-mirror-test-{}", uuid::Uuid::new_v4()));
        let projects_dir = temp.join("projects");
        let mut env = std::collections::HashMap::new();
        env.insert(
            "CLAUDE_CONFIG_DIR".to_string(),
            temp.to_string_lossy().to_string(),
        );
        let options = ClaudeAgentOptions::builder()
            .env(env)
            .session_store(store.clone())
            .build();
        let mut batcher = TranscriptMirrorBatcher::from_options(&options).expect("batcher");

        batcher
            .enqueue_value(&mirror_frame(&projects_dir, "1"))
            .await
            .unwrap();
        batcher
            .enqueue_value(&mirror_frame(&projects_dir, "2"))
            .await
            .unwrap();
        assert_eq!(batcher.pending_entries(), 2);

        let errors = batcher.flush().await.unwrap();
        assert!(errors.is_empty());

        let entries = store
            .load(SessionKey {
                project_key: "proj".to_string(),
                session_id: "session-1".to_string(),
                subpath: None,
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["uuid"], "1");
        assert_eq!(entries[1]["uuid"], "2");
    }

    #[tokio::test]
    async fn batcher_reports_store_failures_as_nonfatal_mirror_errors() {
        let temp = std::env::temp_dir().join(format!(
            "claude-rust-mirror-error-test-{}",
            uuid::Uuid::new_v4()
        ));
        let projects_dir = temp.join("projects");
        let mut env = std::collections::HashMap::new();
        env.insert(
            "CLAUDE_CONFIG_DIR".to_string(),
            temp.to_string_lossy().to_string(),
        );
        let options = ClaudeAgentOptions::builder()
            .env(env)
            .session_store(FailingStore)
            .build();
        let mut batcher = TranscriptMirrorBatcher::from_options(&options).expect("batcher");

        batcher
            .enqueue_value(&mirror_frame(&projects_dir, "1"))
            .await
            .unwrap();
        let errors = batcher.flush().await.unwrap();

        assert_eq!(errors.len(), 1);
        match &errors[0] {
            Message::MirrorErrorMsg(message) => {
                assert!(message.error.contains("store failed"));
                assert_eq!(
                    message.key.as_ref().and_then(|key| key.get("project_key")),
                    Some(&serde_json::json!("proj"))
                );
            }
            other => panic!("expected mirror_error system message, got {other:?}"),
        }
    }
}
