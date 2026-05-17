use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use roder_api::conversation::{ConversationItem, TurnItem};
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
        let file = fs::File::open(&file_path)
            .await
            .with_context(|| format!("open turn item log {}", file_path.display()))?;
        let reader = BufReader::new(file);
        let mut lines = reader.lines();
        let mut turns: Vec<TurnRecord> = Vec::new();
        let mut line_number = 0usize;
        while let Some(line) = lines
            .next_line()
            .await
            .with_context(|| format!("read turn item log {}", file_path.display()))?
        {
            line_number += 1;
            if line.trim().is_empty() {
                continue;
            }
            let stream = serde_json::Deserializer::from_str(&line).into_iter::<PersistedTurnItem>();
            for persisted in stream {
                let persisted = persisted.with_context(|| {
                    format!(
                        "parse turn item record in {}:{}",
                        file_path.display(),
                        line_number
                    )
                })?;
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
        fs::create_dir_all(&dir)
            .await
            .with_context(|| format!("create session directory {}", dir.display()))?;
        let metadata_path = dir.join("metadata.json");
        fs::write(
            &metadata_path,
            serde_json::to_vec_pretty(&metadata).context("serialize session metadata")?,
        )
        .await
        .with_context(|| format!("write session metadata {}", metadata_path.display()))?;
        Ok(metadata)
    }

    async fn list_sessions(&self) -> anyhow::Result<Vec<SessionMetadata>> {
        if !self.base_path.exists() {
            return Ok(Vec::new());
        }
        let mut entries = fs::read_dir(&self.base_path)
            .await
            .with_context(|| format!("read session directory {}", self.base_path.display()))?;
        let mut sessions = Vec::new();
        while let Some(entry) = entries
            .next_entry()
            .await
            .with_context(|| format!("read session entry under {}", self.base_path.display()))?
        {
            let metadata_path = entry.path().join("metadata.json");
            if metadata_path.exists() {
                let data = fs::read(&metadata_path).await.with_context(|| {
                    format!("read session metadata {}", metadata_path.display())
                })?;
                let metadata =
                    serde_json::from_slice::<SessionMetadata>(&data).with_context(|| {
                        format!("parse session metadata {}", metadata_path.display())
                    })?;
                sessions.push(self.with_derived_title(metadata).await?);
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
                let data = fs::read(&path)
                    .await
                    .with_context(|| format!("read session metadata {}", path.display()))?;
                let metadata = serde_json::from_slice::<SessionMetadata>(&data)
                    .with_context(|| format!("parse session metadata {}", path.display()))?;
                Some(self.with_derived_title(metadata).await?)
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
        fs::create_dir_all(&dir)
            .await
            .with_context(|| format!("create session directory {}", dir.display()))?;
        let file_path = dir.join("events.jsonl");
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file_path)
            .await
            .with_context(|| format!("open event log {}", file_path.display()))?;
        let mut line = serde_json::to_vec(envelope).context("serialize event envelope")?;
        line.push(b'\n');
        file.write_all(&line)
            .await
            .with_context(|| format!("append event record to {}", file_path.display()))?;
        Ok(())
    }

    async fn append_turn_item(
        &self,
        thread_id: &ThreadId,
        turn_id: &TurnId,
        item: &TurnItem,
    ) -> anyhow::Result<()> {
        let dir = self.session_dir(thread_id);
        fs::create_dir_all(&dir)
            .await
            .with_context(|| format!("create session directory {}", dir.display()))?;
        let file_path = dir.join("turn_items.jsonl");
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file_path)
            .await
            .with_context(|| format!("open turn item log {}", file_path.display()))?;
        let persisted = PersistedTurnItem {
            turn_id: turn_id.clone(),
            timestamp: OffsetDateTime::now_utc(),
            item: item.clone(),
        };
        let mut line = serde_json::to_vec(&persisted).context("serialize turn item record")?;
        line.push(b'\n');
        file.write_all(&line)
            .await
            .with_context(|| format!("append turn item record to {}", file_path.display()))?;
        self.update_metadata_for_turn_item(thread_id, item).await?;
        Ok(())
    }
}

impl JsonlSessionStore {
    async fn load_events(&self, thread_id: &ThreadId) -> anyhow::Result<Vec<EventEnvelope>> {
        let file_path = self.session_dir(thread_id).join("events.jsonl");
        if !file_path.exists() {
            return Ok(Vec::new());
        }
        let file = fs::File::open(&file_path)
            .await
            .with_context(|| format!("open event log {}", file_path.display()))?;
        let reader = BufReader::new(file);
        let mut lines = reader.lines();
        let mut events = Vec::new();
        let mut line_number = 0usize;
        while let Some(line) = lines
            .next_line()
            .await
            .with_context(|| format!("read event log {}", file_path.display()))?
        {
            line_number += 1;
            if line.trim().is_empty() {
                continue;
            }
            let stream = serde_json::Deserializer::from_str(&line).into_iter::<EventEnvelope>();
            for event in stream {
                events.push(event.with_context(|| {
                    format!(
                        "parse event record in {}:{}",
                        file_path.display(),
                        line_number
                    )
                })?);
            }
        }
        Ok(events)
    }

    async fn update_metadata_for_turn_item(
        &self,
        thread_id: &ThreadId,
        item: &TurnItem,
    ) -> anyhow::Result<()> {
        let metadata_path = self.session_dir(thread_id).join("metadata.json");
        if !metadata_path.exists() {
            return Ok(());
        }
        let data = fs::read(&metadata_path)
            .await
            .with_context(|| format!("read session metadata {}", metadata_path.display()))?;
        let mut metadata = serde_json::from_slice::<SessionMetadata>(&data)
            .with_context(|| format!("parse session metadata {}", metadata_path.display()))?;
        metadata.updated_at = OffsetDateTime::now_utc();
        if matches!(
            item,
            ConversationItem::UserMessage(_) | ConversationItem::AssistantMessage(_)
        ) {
            metadata.message_count = metadata.message_count.saturating_add(1);
        }
        if metadata
            .title
            .as_ref()
            .is_none_or(|title| title.trim().is_empty())
        {
            if let ConversationItem::UserMessage(message) = item {
                metadata.title = title_from_user_text(&message.text);
            }
        }
        fs::write(
            &metadata_path,
            serde_json::to_vec_pretty(&metadata).context("serialize session metadata")?,
        )
        .await
        .with_context(|| format!("write session metadata {}", metadata_path.display()))?;
        Ok(())
    }

    async fn with_derived_title(
        &self,
        mut metadata: SessionMetadata,
    ) -> anyhow::Result<SessionMetadata> {
        if metadata
            .title
            .as_ref()
            .is_some_and(|title| !title.trim().is_empty())
        {
            return Ok(metadata);
        }
        for turn in self.read_turns(&metadata.thread_id).await? {
            for item in turn.items {
                if let ConversationItem::UserMessage(message) = item {
                    if let Some(title) = title_from_user_text(&message.text) {
                        metadata.title = Some(title);
                        return Ok(metadata);
                    }
                }
            }
        }
        Ok(metadata)
    }
}

fn title_from_user_text(text: &str) -> Option<String> {
    let folded = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if folded.is_empty() {
        None
    } else {
        Some(truncate_chars(&folded, 72))
    }
}

fn truncate_chars(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    if max <= 3 {
        return value.chars().take(max).collect();
    }
    let mut out = value.chars().take(max - 3).collect::<String>();
    out.push_str("...");
    out
}

#[derive(serde::Serialize, serde::Deserialize)]
struct PersistedTurnItem {
    turn_id: TurnId,
    #[serde(with = "time::serde::rfc3339")]
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
                &ConversationItem::UserMessage(UserMessage::text("hello")),
            )
            .await
            .unwrap();
        store
            .append_turn_item(
                &thread_id,
                &turn_id,
                &ConversationItem::AssistantMessage(AssistantMessage {
                    text: "world".to_string(),
                    phase: None,
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
    async fn load_session_preserves_provider_metadata_with_encrypted_reasoning() {
        let base_path =
            std::env::temp_dir().join(format!("roder-jsonl-session-test-{}", uuid::Uuid::new_v4()));
        let store = JsonlSessionStore {
            base_path: base_path.clone(),
        };
        let thread_id = "thread-encrypted-reasoning".to_string();
        let turn_id = "turn-encrypted-reasoning".to_string();
        let now = OffsetDateTime::UNIX_EPOCH;
        let metadata = serde_json::json!({
            "id": "resp_1",
            "output": [{
                "id": "rs_1",
                "type": "reasoning",
                "encrypted_content": "opaque-thinking-state",
                "summary": []
            }]
        });

        store
            .create_session(SessionMetadata {
                thread_id: thread_id.clone(),
                title: Some("Resume encrypted reasoning".to_string()),
                workspace: None,
                provider: Some("openai".to_string()),
                model: Some("gpt-5.5".to_string()),
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
                &ConversationItem::ProviderMetadata(metadata.clone()),
            )
            .await
            .unwrap();

        let snapshot = store.load_session(&thread_id).await.unwrap().unwrap();

        assert_eq!(
            snapshot.turns[0].items[0],
            ConversationItem::ProviderMetadata(metadata)
        );

        let _ = fs::remove_dir_all(base_path).await;
    }

    #[tokio::test]
    async fn append_turn_item_updates_metadata_counts_and_recency() {
        let base_path = std::env::temp_dir().join(format!(
            "roder-jsonl-metadata-test-{}",
            uuid::Uuid::new_v4()
        ));
        let store = JsonlSessionStore {
            base_path: base_path.clone(),
        };
        let thread_id = "thread-metadata".to_string();
        let turn_id = "turn-metadata".to_string();
        let now = OffsetDateTime::UNIX_EPOCH;

        store
            .create_session(SessionMetadata {
                thread_id: thread_id.clone(),
                title: Some("Metadata".to_string()),
                workspace: None,
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
                &ConversationItem::UserMessage(UserMessage::text("hello")),
            )
            .await
            .unwrap();
        store
            .append_turn_item(
                &thread_id,
                &turn_id,
                &ConversationItem::ProviderMetadata(serde_json::json!({"id": "resp_1"})),
            )
            .await
            .unwrap();
        store
            .append_turn_item(
                &thread_id,
                &turn_id,
                &ConversationItem::AssistantMessage(AssistantMessage {
                    text: "world".to_string(),
                    phase: None,
                }),
            )
            .await
            .unwrap();

        let sessions = store.list_sessions().await.unwrap();

        assert_eq!(sessions[0].message_count, 2);
        assert!(sessions[0].updated_at > now);

        let _ = fs::remove_dir_all(base_path).await;
    }

    #[tokio::test]
    async fn first_user_message_names_untitled_session() {
        let base_path =
            std::env::temp_dir().join(format!("roder-jsonl-title-test-{}", uuid::Uuid::new_v4()));
        let store = JsonlSessionStore {
            base_path: base_path.clone(),
        };
        let thread_id = "thread-title".to_string();
        let turn_id = "turn-title".to_string();
        let now = OffsetDateTime::UNIX_EPOCH;

        store
            .create_session(SessionMetadata {
                thread_id: thread_id.clone(),
                title: None,
                workspace: Some("/workspace/gode".to_string()),
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
                &ConversationItem::UserMessage(UserMessage::text(
                    "please make resume sessions easier to find",
                )),
            )
            .await
            .unwrap();

        let sessions = store.list_sessions().await.unwrap();
        let snapshot = store.load_session(&thread_id).await.unwrap().unwrap();

        assert_eq!(
            sessions[0].title.as_deref(),
            Some("please make resume sessions easier to find")
        );
        assert_eq!(
            snapshot.metadata.unwrap().title.as_deref(),
            Some("please make resume sessions easier to find")
        );

        let _ = fs::remove_dir_all(base_path).await;
    }

    #[tokio::test]
    async fn load_session_accepts_concatenated_jsonl_records() {
        let base_path =
            std::env::temp_dir().join(format!("roder-jsonl-concat-test-{}", uuid::Uuid::new_v4()));
        let store = JsonlSessionStore {
            base_path: base_path.clone(),
        };
        let thread_id = "thread-concat".to_string();
        let turn_id = "turn-concat".to_string();
        let now = OffsetDateTime::UNIX_EPOCH;

        store
            .create_session(SessionMetadata {
                thread_id: thread_id.clone(),
                title: Some("Concatenated jsonl".to_string()),
                workspace: None,
                provider: Some("mock".to_string()),
                model: Some("mock".to_string()),
                created_at: now,
                updated_at: now,
                message_count: 0,
            })
            .await
            .unwrap();

        let dir = store.session_dir(&thread_id);
        let first = PersistedTurnItem {
            turn_id: turn_id.clone(),
            timestamp: now,
            item: ConversationItem::UserMessage(UserMessage::text("first")),
        };
        let second = PersistedTurnItem {
            turn_id: turn_id.clone(),
            timestamp: now,
            item: ConversationItem::AssistantMessage(AssistantMessage {
                text: "second".to_string(),
                phase: None,
            }),
        };
        let concatenated = format!(
            "{}{}\n",
            serde_json::to_string(&first).unwrap(),
            serde_json::to_string(&second).unwrap()
        );
        fs::write(dir.join("turn_items.jsonl"), concatenated)
            .await
            .unwrap();

        let snapshot = store.load_session(&thread_id).await.unwrap().unwrap();

        assert_eq!(snapshot.turns.len(), 1);
        assert_eq!(snapshot.turns[0].items.len(), 2);

        let _ = fs::remove_dir_all(base_path).await;
    }

    #[tokio::test]
    async fn malformed_turn_item_reports_file_and_line() {
        let base_path = std::env::temp_dir().join(format!(
            "roder-jsonl-malformed-test-{}",
            uuid::Uuid::new_v4()
        ));
        let store = JsonlSessionStore {
            base_path: base_path.clone(),
        };
        let thread_id = "thread-malformed".to_string();
        let now = OffsetDateTime::UNIX_EPOCH;

        store
            .create_session(SessionMetadata {
                thread_id: thread_id.clone(),
                title: Some("Malformed jsonl".to_string()),
                workspace: None,
                provider: Some("mock".to_string()),
                model: Some("mock".to_string()),
                created_at: now,
                updated_at: now,
                message_count: 0,
            })
            .await
            .unwrap();
        fs::write(
            store.session_dir(&thread_id).join("turn_items.jsonl"),
            "{\"turn_id\":\"broken\"\n",
        )
        .await
        .unwrap();

        let err = store.load_session(&thread_id).await.unwrap_err();
        let rendered = format!("{err:#}");

        assert!(rendered.contains("parse turn item record in"));
        assert!(rendered.contains("turn_items.jsonl:1"));

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
