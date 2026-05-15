use std::path::PathBuf;
use std::sync::Arc;

use roder_api::conversation::TurnItem;
use roder_api::events::{EventEnvelope, ThreadId, TurnId};
use roder_api::session::{
    SessionMetadata, SessionStore, SessionStoreFactory, ThreadSnapshot, TurnRecord,
};
use time::OffsetDateTime;
use tokio::fs::{self, OpenOptions};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

pub struct JsonlSessionStore {
    pub base_path: PathBuf,
}

impl JsonlSessionStore {
    fn session_dir(&self, thread_id: &ThreadId) -> PathBuf {
        self.base_path.join(thread_id)
    }

    async fn read_turns(&self, thread_id: &ThreadId) -> anyhow::Result<Vec<TurnRecord>> {
        let file_path = self.session_dir(thread_id).join("turn_items.jsonl");
        if !file_path.exists() {
            return Ok(Vec::new());
        }
        let file = fs::File::open(&file_path).await?;
        let reader = BufReader::new(file);
        let mut lines = reader.lines();
        let mut turns: Vec<TurnRecord> = Vec::new();
        while let Some(line) = lines.next_line().await? {
            if line.trim().is_empty() {
                continue;
            }
            let persisted: PersistedTurnItem = serde_json::from_str(&line)?;
            if let Some(turn) = turns
                .iter_mut()
                .find(|turn| turn.turn_id == persisted.turn_id)
            {
                turn.items.push(persisted.item);
            } else {
                turns.push(TurnRecord {
                    thread_id: thread_id.clone(),
                    turn_id: persisted.turn_id,
                    items: vec![persisted.item],
                    created_at: persisted.timestamp,
                    completed_at: None,
                });
            }
        }
        Ok(turns)
    }
}

#[async_trait::async_trait]
impl SessionStore for JsonlSessionStore {
    fn id(&self) -> roder_api::session::SessionStoreId {
        "jsonl".to_string()
    }

    async fn create_session(&self, metadata: SessionMetadata) -> anyhow::Result<SessionMetadata> {
        let dir = self.session_dir(&metadata.thread_id);
        fs::create_dir_all(&dir).await?;
        let metadata_path = dir.join("metadata.json");
        fs::write(metadata_path, serde_json::to_vec_pretty(&metadata)?).await?;
        Ok(metadata)
    }

    async fn list_sessions(&self) -> anyhow::Result<Vec<SessionMetadata>> {
        if !self.base_path.exists() {
            return Ok(Vec::new());
        }
        let mut entries = fs::read_dir(&self.base_path).await?;
        let mut sessions = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            let metadata_path = entry.path().join("metadata.json");
            if metadata_path.exists() {
                let data = fs::read(metadata_path).await?;
                sessions.push(serde_json::from_slice::<SessionMetadata>(&data)?);
            }
        }
        sessions.sort_by_key(|session| std::cmp::Reverse(session.updated_at));
        Ok(sessions)
    }

    async fn load_session(&self, thread_id: &ThreadId) -> anyhow::Result<Option<ThreadSnapshot>> {
        let dir = self.session_dir(thread_id);
        if !dir.exists() {
            return Ok(None);
        }
        let metadata = {
            let path = dir.join("metadata.json");
            if path.exists() {
                Some(serde_json::from_slice::<SessionMetadata>(
                    &fs::read(path).await?,
                )?)
            } else {
                None
            }
        };
        let events = self.load_events(thread_id).await?;
        let turns = self.read_turns(thread_id).await?;
        Ok(Some(ThreadSnapshot {
            metadata,
            events,
            turns,
        }))
    }

    async fn append_event(
        &self,
        thread_id: &ThreadId,
        envelope: &EventEnvelope,
    ) -> anyhow::Result<()> {
        let dir = self.session_dir(thread_id);
        fs::create_dir_all(&dir).await?;
        let file_path = dir.join("events.jsonl");
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file_path)
            .await?;
        file.write_all(serde_json::to_string(envelope)?.as_bytes())
            .await?;
        file.write_all(b"\n").await?;
        Ok(())
    }

    async fn append_turn_item(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        item: &TurnItem,
    ) -> anyhow::Result<()> {
        let dir = self.session_dir(thread_id);
        fs::create_dir_all(&dir).await?;
        let file_path = dir.join("turn_items.jsonl");
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file_path)
            .await?;
        let persisted = PersistedTurnItem {
            turn_id: turn_id.clone(),
            timestamp: OffsetDateTime::now_utc(),
            item: item.clone(),
        };
        file.write_all(serde_json::to_string(&persisted)?.as_bytes())
            .await?;
        file.write_all(b"\n").await?;
        Ok(())
    }
}

impl JsonlSessionStore {
    async fn load_events(&self, thread_id: &ThreadId) -> anyhow::Result<Vec<EventEnvelope>> {
        let file_path = self.session_dir(thread_id).join("events.jsonl");
        if !file_path.exists() {
            return Ok(Vec::new());
        }
        let file = fs::File::open(&file_path).await?;
        let reader = BufReader::new(file);
        let mut lines = reader.lines();
        let mut events = Vec::new();
        while let Some(line) = lines.next_line().await? {
            if line.trim().is_empty() {
                continue;
            }
            events.push(serde_json::from_str::<EventEnvelope>(&line)?);
        }
        Ok(events)
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct PersistedTurnItem {
    turn_id: TurnId,
    timestamp: OffsetDateTime,
    item: TurnItem,
}

pub struct JsonlSessionStoreFactory {
    pub base_path: PathBuf,
}

impl SessionStoreFactory for JsonlSessionStoreFactory {
    fn id(&self) -> roder_api::session::SessionStoreId {
        "jsonl".to_string()
    }

    fn create(&self) -> Arc<dyn SessionStore> {
        Arc::new(JsonlSessionStore {
            base_path: self.base_path.clone(),
        })
    }
}
