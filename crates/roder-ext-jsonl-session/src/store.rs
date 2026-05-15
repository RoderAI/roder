use std::path::PathBuf;
use std::sync::Arc;

use roder_api::conversation::TurnItem;
use roder_api::events::{EventEnvelope, RoderEvent, ThreadId, TurnId};
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
        let mut turns = self.read_turns(thread_id).await?;
        project_turn_completion(&mut turns, &events);
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

fn project_turn_completion(turns: &mut [TurnRecord], events: &[EventEnvelope]) {
    for envelope in events {
        let (turn_id, timestamp) = match &envelope.event {
            RoderEvent::TurnCompleted(event) => (&event.turn_id, event.timestamp),
            RoderEvent::TurnFailed(event) => (&event.turn_id, event.timestamp),
            RoderEvent::TurnInterrupted(event) => (&event.turn_id, event.timestamp),
            _ => continue,
        };
        if let Some(turn) = turns.iter_mut().find(|turn| &turn.turn_id == turn_id) {
            turn.completed_at = Some(timestamp);
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::conversation::{AssistantMessage, ConversationItem, UserMessage};
    use roder_api::events::{EventSource, TurnCompleted};

    #[tokio::test]
    async fn load_session_projects_turn_items_and_completion() {
        let base_path =
            std::env::temp_dir().join(format!("roder-jsonl-session-test-{}", uuid::Uuid::new_v4()));
        let store = JsonlSessionStore {
            base_path: base_path.clone(),
        };
        let thread_id = "thread-a".to_string();
        let turn_id = "turn-a".to_string();
        let now = OffsetDateTime::UNIX_EPOCH;

        store
            .create_session(SessionMetadata {
                thread_id: thread_id.clone(),
                title: Some("Resume me".to_string()),
                workspace: Some("/workspace".to_string()),
                provider: Some("mock".to_string()),
                model: Some("mock".to_string()),
                created_at: now,
                updated_at: now,
                message_count: 0,
            })
            .await
            .unwrap();
        store
            .append_turn_item(
                &thread_id,
                &turn_id,
                &ConversationItem::UserMessage(UserMessage {
                    text: "hello".to_string(),
                }),
            )
            .await
            .unwrap();
        store
            .append_turn_item(
                &thread_id,
                &turn_id,
                &ConversationItem::AssistantMessage(AssistantMessage {
                    text: "world".to_string(),
                }),
            )
            .await
            .unwrap();
        store
            .append_event(
                &thread_id,
                &EventEnvelope {
                    event_id: "event-a".to_string(),
                    seq: 1,
                    timestamp: now,
                    source: EventSource::Core,
                    kind: "turn.completed".to_string(),
                    thread_id: Some(thread_id.clone()),
                    turn_id: Some(turn_id.clone()),
                    event: RoderEvent::TurnCompleted(TurnCompleted {
                        thread_id: thread_id.clone(),
                        turn_id: turn_id.clone(),
                        timestamp: now,
                    }),
                },
            )
            .await
            .unwrap();

        let snapshot = store.load_session(&thread_id).await.unwrap().unwrap();

        assert_eq!(
            snapshot.metadata.unwrap().title.as_deref(),
            Some("Resume me")
        );
        assert_eq!(snapshot.events.len(), 1);
        assert_eq!(snapshot.turns.len(), 1);
        assert_eq!(snapshot.turns[0].turn_id, turn_id);
        assert_eq!(snapshot.turns[0].items.len(), 2);
        assert_eq!(snapshot.turns[0].completed_at, Some(now));

        let _ = fs::remove_dir_all(base_path).await;
    }

    #[tokio::test]
    async fn load_missing_session_returns_none() {
        let base_path = std::env::temp_dir().join(format!(
            "roder-jsonl-missing-session-test-{}",
            uuid::Uuid::new_v4()
        ));
        let store = JsonlSessionStore {
            base_path: base_path.clone(),
        };

        assert!(
            store
                .load_session(&"missing".to_string())
                .await
                .unwrap()
                .is_none()
        );

        let _ = fs::remove_dir_all(base_path).await;
    }
}
