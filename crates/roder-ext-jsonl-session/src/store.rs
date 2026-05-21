use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use roder_api::conversation::{ConversationItem, TurnItem};
use roder_api::events::{EventEnvelope, RoderEvent, ThreadId, TurnId};
use roder_api::extension_state::ExtensionStateRecord;
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

    fn archived_session_dir(&self, thread_id: &ThreadId) -> PathBuf {
        archived_sessions_root(&self.base_path).join(thread_id)
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

    fn local_session_root(&self) -> Option<PathBuf> {
        Some(self.base_path.clone())
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

    async fn update_session_metadata(
        &self,
        metadata: SessionMetadata,
    ) -> anyhow::Result<SessionMetadata> {
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
            let file_type = entry.file_type().await.with_context(|| {
                format!("read session entry type under {}", self.base_path.display())
            })?;
            if file_type.is_dir() {
                let thread_id = entry.file_name().to_string_lossy().to_string();
                let metadata = self
                    .load_or_infer_metadata(&entry.path(), &thread_id)
                    .await?;
                sessions.push(metadata);
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
        let metadata = Some(self.load_or_infer_metadata(&dir, thread_id).await?);
        let events = self.load_events(thread_id).await?;
        let mut turns = self.read_turns(thread_id).await?;
        let extension_states = self.load_extension_states(thread_id).await?;
        project_turn_completion(&mut turns, &events);
        Ok(Some(ThreadSnapshot {
            metadata,
            events,
            turns,
            extension_states,
        }))
    }

    async fn archive_session(&self, thread_id: &ThreadId) -> anyhow::Result<bool> {
        let source = self.session_dir(thread_id);
        if !source.exists() {
            return Ok(false);
        }
        let archive_root = archived_sessions_root(&self.base_path);
        fs::create_dir_all(&archive_root).await.with_context(|| {
            format!(
                "create archived sessions directory {}",
                archive_root.display()
            )
        })?;
        let destination = self.archived_session_dir(thread_id);
        if destination.exists() {
            anyhow::bail!("archived session already exists: {}", destination.display());
        }
        fs::rename(&source, &destination).await.with_context(|| {
            format!(
                "archive session {} from {} to {}",
                thread_id,
                source.display(),
                destination.display()
            )
        })?;
        Ok(true)
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

    async fn append_extension_state(
        &self,
        thread_id: &ThreadId,
        record: &ExtensionStateRecord,
    ) -> anyhow::Result<()> {
        let dir = self.session_dir(thread_id);
        fs::create_dir_all(&dir)
            .await
            .with_context(|| format!("create session directory {}", dir.display()))?;
        let file_path = dir.join("extension_state.jsonl");
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file_path)
            .await
            .with_context(|| format!("open extension state log {}", file_path.display()))?;
        let mut line = serde_json::to_vec(record).context("serialize extension state record")?;
        line.push(b'\n');
        file.write_all(&line)
            .await
            .with_context(|| format!("append extension state record to {}", file_path.display()))?;
        Ok(())
    }
}

impl JsonlSessionStore {
    async fn load_extension_states(
        &self,
        thread_id: &ThreadId,
    ) -> anyhow::Result<Vec<ExtensionStateRecord>> {
        let file_path = self.session_dir(thread_id).join("extension_state.jsonl");
        if !file_path.exists() {
            return Ok(Vec::new());
        }
        let file = fs::File::open(&file_path)
            .await
            .with_context(|| format!("open extension state log {}", file_path.display()))?;
        let reader = BufReader::new(file);
        let mut lines = reader.lines();
        let mut records = Vec::new();
        let mut line_number = 0usize;
        while let Some(line) = lines
            .next_line()
            .await
            .with_context(|| format!("read extension state log {}", file_path.display()))?
        {
            line_number += 1;
            if line.trim().is_empty() {
                continue;
            }
            let stream =
                serde_json::Deserializer::from_str(&line).into_iter::<ExtensionStateRecord>();
            for record in stream {
                records.push(record.with_context(|| {
                    format!(
                        "parse extension state record in {}:{}",
                        file_path.display(),
                        line_number
                    )
                })?);
            }
        }
        Ok(records)
    }

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
        let session_dir = self.session_dir(thread_id);
        let (mut metadata, count_current_item) = if metadata_path.exists() {
            let data = fs::read(&metadata_path)
                .await
                .with_context(|| format!("read session metadata {}", metadata_path.display()))?;
            match parse_metadata_tolerant(&data) {
                Ok((metadata, _needs_repair)) => (self.with_derived_title(metadata).await?, true),
                Err(_) => (self.infer_metadata(&session_dir, thread_id).await, false),
            }
        } else {
            (self.infer_metadata(&session_dir, thread_id).await, false)
        };
        metadata.updated_at = OffsetDateTime::now_utc();
        if count_current_item
            && matches!(
                item,
                ConversationItem::UserMessage(_) | ConversationItem::AssistantMessage(_)
            )
        {
            metadata.message_count = metadata.message_count.saturating_add(1);
        }
        if metadata
            .title
            .as_ref()
            .is_none_or(|title| title.trim().is_empty())
            && let ConversationItem::UserMessage(message) = item
        {
            metadata.title = title_from_user_text(&message.text);
        }
        fs::write(
            &metadata_path,
            serde_json::to_vec_pretty(&metadata).context("serialize session metadata")?,
        )
        .await
        .with_context(|| format!("write session metadata {}", metadata_path.display()))?;
        Ok(())
    }

    async fn load_or_infer_metadata(
        &self,
        dir: &Path,
        thread_id: &ThreadId,
    ) -> anyhow::Result<SessionMetadata> {
        let metadata_path = dir.join("metadata.json");
        if metadata_path.exists() {
            let data = fs::read(&metadata_path)
                .await
                .with_context(|| format!("read session metadata {}", metadata_path.display()))?;
            match parse_metadata_tolerant(&data) {
                Ok((metadata, needs_repair)) => {
                    let metadata = self.with_derived_title(metadata).await?;
                    if needs_repair {
                        self.repair_metadata_file(&metadata_path, &metadata).await;
                    }
                    return Ok(metadata);
                }
                Err(_) => {
                    let metadata = self.infer_metadata(dir, thread_id).await;
                    self.repair_metadata_file(&metadata_path, &metadata).await;
                    return Ok(metadata);
                }
            }
        }

        let metadata = self.infer_metadata(dir, thread_id).await;
        self.repair_metadata_file(&metadata_path, &metadata).await;
        Ok(metadata)
    }

    async fn repair_metadata_file(&self, metadata_path: &Path, metadata: &SessionMetadata) {
        let Ok(serialized) = serde_json::to_vec_pretty(metadata) else {
            return;
        };
        let Some(parent) = metadata_path.parent() else {
            return;
        };
        if fs::create_dir_all(parent).await.is_err() {
            return;
        }
        let _ = fs::write(metadata_path, serialized).await;
    }

    async fn infer_metadata(&self, dir: &Path, thread_id: &ThreadId) -> SessionMetadata {
        let mut title = None;
        let mut provider = None;
        let mut model = None;
        let mut created_at = None;
        let mut updated_at = None;
        let mut message_count = 0u32;

        if let Ok(turns) = self.read_turns(thread_id).await {
            for turn in turns {
                track_timestamp(&mut created_at, &mut updated_at, turn.created_at);
                if let Some(completed_at) = turn.completed_at {
                    track_timestamp(&mut created_at, &mut updated_at, completed_at);
                }
                for item in turn.items {
                    match item {
                        ConversationItem::UserMessage(message) => {
                            message_count = message_count.saturating_add(1);
                            if title.is_none() {
                                title = title_from_user_text(&message.text);
                            }
                        }
                        ConversationItem::AssistantMessage(_) => {
                            message_count = message_count.saturating_add(1);
                        }
                        ConversationItem::ProviderMetadata(metadata) => {
                            if provider.is_none() {
                                provider = metadata_string_field(&metadata, "provider")
                                    .or_else(|| metadata_string_field(&metadata, "provider_id"));
                            }
                            if model.is_none() {
                                model = metadata_string_field(&metadata, "model");
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        if let Ok(events) = self.load_events(thread_id).await {
            for envelope in events {
                track_timestamp(&mut created_at, &mut updated_at, envelope.timestamp);
            }
        }

        if (created_at.is_none() || updated_at.is_none())
            && let Some(modified_at) = modified_at(dir).await
        {
            track_timestamp(&mut created_at, &mut updated_at, modified_at);
        }

        let fallback_time = OffsetDateTime::now_utc();
        SessionMetadata {
            thread_id: thread_id.clone(),
            title,
            workspace: None,
            provider,
            model,
            runner_destination: None,
            runner_state: None,
            created_at: created_at.unwrap_or(fallback_time),
            updated_at: updated_at.unwrap_or(fallback_time),
            message_count,
        }
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
        let Ok(turns) = self.read_turns(&metadata.thread_id).await else {
            return Ok(metadata);
        };
        for turn in turns {
            for item in turn.items {
                if let ConversationItem::UserMessage(message) = item
                    && let Some(title) = title_from_user_text(&message.text)
                {
                    metadata.title = Some(title);
                    return Ok(metadata);
                }
            }
        }
        Ok(metadata)
    }
}

fn parse_metadata_tolerant(data: &[u8]) -> serde_json::Result<(SessionMetadata, bool)> {
    match serde_json::from_slice::<SessionMetadata>(data) {
        Ok(metadata) => Ok((metadata, false)),
        Err(strict_err) => {
            let mut deserializer = serde_json::Deserializer::from_slice(data);
            match serde::Deserialize::deserialize(&mut deserializer) {
                Ok(metadata) => Ok((metadata, true)),
                Err(_) => Err(strict_err),
            }
        }
    }
}

fn metadata_string_field(value: &serde_json::Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
}

async fn modified_at(path: &Path) -> Option<OffsetDateTime> {
    fs::metadata(path)
        .await
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .map(OffsetDateTime::from)
}

fn track_timestamp(
    created_at: &mut Option<OffsetDateTime>,
    updated_at: &mut Option<OffsetDateTime>,
    timestamp: OffsetDateTime,
) {
    if created_at.is_none_or(|created| timestamp < created) {
        *created_at = Some(timestamp);
    }
    if updated_at.is_none_or(|updated| timestamp > updated) {
        *updated_at = Some(timestamp);
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

fn archived_sessions_root(active_sessions_root: &Path) -> PathBuf {
    active_sessions_root
        .parent()
        .map(|parent| parent.join("archived_sessions"))
        .unwrap_or_else(|| active_sessions_root.with_file_name("archived_sessions"))
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
    use roder_api::events::{EventSource, SubagentTraceCreated, TurnCompleted};
    use roder_api::trace::{
        ParentTurnRef, SubagentDestination, SubagentDestinationKind, SubagentTraceStatus,
        SubagentTraceSummary,
    };

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
                runner_destination: None,
                runner_state: None,
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
    async fn archive_session_moves_session_out_of_active_list() {
        let base_path =
            std::env::temp_dir().join(format!("roder-jsonl-archive-test-{}", uuid::Uuid::new_v4()));
        let store = JsonlSessionStore {
            base_path: base_path.clone(),
        };
        let thread_id = "thread-archive".to_string();
        let now = OffsetDateTime::UNIX_EPOCH;

        store
            .create_session(SessionMetadata {
                thread_id: thread_id.clone(),
                title: Some("Archive me".to_string()),
                workspace: Some("/workspace".to_string()),
                provider: Some("mock".to_string()),
                model: Some("mock".to_string()),
                runner_destination: None,
                runner_state: None,
                created_at: now,
                updated_at: now,
                message_count: 0,
            })
            .await
            .unwrap();

        assert!(store.archive_session(&thread_id).await.unwrap());
        assert!(store.list_sessions().await.unwrap().is_empty());
        assert!(store.load_session(&thread_id).await.unwrap().is_none());
        assert!(
            base_path
                .parent()
                .unwrap()
                .join("archived_sessions")
                .join(&thread_id)
                .join("metadata.json")
                .exists()
        );

        let _ = fs::remove_dir_all(base_path.parent().unwrap().join("archived_sessions")).await;
        let _ = fs::remove_dir_all(base_path).await;
    }

    #[tokio::test]
    async fn extension_state_round_trips_through_thread_snapshot() {
        let base_path = std::env::temp_dir().join(format!(
            "roder-jsonl-extension-state-{}",
            uuid::Uuid::new_v4()
        ));
        let store = JsonlSessionStore {
            base_path: base_path.clone(),
        };
        let thread_id = "thread-state".to_string();
        let now = OffsetDateTime::UNIX_EPOCH;

        store
            .create_session(SessionMetadata {
                thread_id: thread_id.clone(),
                title: None,
                workspace: None,
                provider: None,
                model: None,
                runner_destination: None,
                runner_state: None,
                created_at: now,
                updated_at: now,
                message_count: 0,
            })
            .await
            .unwrap();
        store
            .append_extension_state(
                &thread_id,
                &roder_api::extension_state::ExtensionStateRecord {
                    extension_id: "demo".to_string(),
                    key: "prefs".to_string(),
                    scope: roder_api::extension_state::ExtensionStoreScope::Thread {
                        thread_id: thread_id.clone(),
                    },
                    schema_version: 2,
                    value: serde_json::json!({ "theme": "dark" }),
                },
            )
            .await
            .unwrap();

        let snapshot = store.load_session(&thread_id).await.unwrap().unwrap();

        assert_eq!(snapshot.extension_states.len(), 1);
        assert_eq!(snapshot.extension_states[0].extension_id, "demo");
        assert_eq!(snapshot.extension_states[0].value["theme"], "dark");

        let _ = fs::remove_dir_all(base_path).await;
    }

    #[tokio::test]
    async fn subagent_trace_events_round_trip_through_thread_snapshot() {
        let base_path = std::env::temp_dir().join(format!(
            "roder-jsonl-subagent-trace-{}",
            uuid::Uuid::new_v4()
        ));
        let store = JsonlSessionStore {
            base_path: base_path.clone(),
        };
        let thread_id = "parent-thread".to_string();
        let turn_id = "parent-turn".to_string();
        let now = OffsetDateTime::UNIX_EPOCH;

        store
            .create_session(SessionMetadata {
                thread_id: thread_id.clone(),
                title: Some("Trace me".to_string()),
                workspace: None,
                provider: Some("mock".to_string()),
                model: Some("mock".to_string()),
                runner_destination: None,
                runner_state: None,
                created_at: now,
                updated_at: now,
                message_count: 0,
            })
            .await
            .unwrap();
        store
            .append_event(
                &thread_id,
                &EventEnvelope {
                    event_id: "trace-event".to_string(),
                    seq: 1,
                    timestamp: now,
                    source: EventSource::Extension,
                    kind: "turn/subagentTraceCreated".to_string(),
                    thread_id: Some(thread_id.clone()),
                    turn_id: Some(turn_id.clone()),
                    event: RoderEvent::SubagentTraceCreated(SubagentTraceCreated {
                        summary: SubagentTraceSummary {
                            trace_id: "trace-1".to_string(),
                            parent: ParentTurnRef {
                                thread_id: thread_id.clone(),
                                turn_id: turn_id.clone(),
                            },
                            child_thread_id: "child-thread".to_string(),
                            child_turn_id: "child-turn".to_string(),
                            title: "Inspect".to_string(),
                            role: "explore".to_string(),
                            model: Some("mock".to_string()),
                            lane: None,
                            status: SubagentTraceStatus::Running,
                            elapsed_ms: 0,
                            usage: None,
                            destination: Some(SubagentDestination {
                                kind: SubagentDestinationKind::InProcess,
                                label: "in-process".to_string(),
                                path: None,
                                provider_id: None,
                                destination_id: None,
                            }),
                            latest_activity: Some("running".to_string()),
                            error_summary: None,
                            exit_reason: None,
                        },
                        timestamp: now,
                    }),
                },
            )
            .await
            .unwrap();

        let snapshot = store.load_session(&thread_id).await.unwrap().unwrap();

        assert_eq!(snapshot.events.len(), 1);
        assert_eq!(
            snapshot.events[0].thread_id.as_deref(),
            Some("parent-thread")
        );
        assert_eq!(snapshot.events[0].turn_id.as_deref(), Some("parent-turn"));
        match &snapshot.events[0].event {
            RoderEvent::SubagentTraceCreated(event) => {
                assert_eq!(event.summary.trace_id, "trace-1");
                assert_eq!(event.summary.child_thread_id, "child-thread");
                assert_eq!(
                    event.summary.destination.as_ref().unwrap().label,
                    "in-process"
                );
            }
            event => panic!("unexpected event: {event:?}"),
        }

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
                runner_destination: None,
                runner_state: None,
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
                runner_destination: None,
                runner_state: None,
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
                runner_destination: None,
                runner_state: None,
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
    async fn list_sessions_recovers_malformed_metadata_from_turn_log() {
        let base_path = std::env::temp_dir().join(format!(
            "roder-jsonl-recover-list-test-{}",
            uuid::Uuid::new_v4()
        ));
        let store = JsonlSessionStore {
            base_path: base_path.clone(),
        };
        let thread_id = "thread-recover-list".to_string();
        let turn_id = "turn-recover-list".to_string();
        let now = OffsetDateTime::UNIX_EPOCH;

        store
            .create_session(SessionMetadata {
                thread_id: thread_id.clone(),
                title: Some("Will be corrupted".to_string()),
                workspace: Some("/workspace".to_string()),
                provider: Some("mock".to_string()),
                model: Some("mock".to_string()),
                runner_destination: None,
                runner_state: None,
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
                &ConversationItem::UserMessage(UserMessage::text("recover this session")),
            )
            .await
            .unwrap();
        store
            .append_turn_item(
                &thread_id,
                &turn_id,
                &ConversationItem::AssistantMessage(AssistantMessage {
                    text: "continuing".to_string(),
                    phase: None,
                }),
            )
            .await
            .unwrap();
        fs::write(
            store.session_dir(&thread_id).join("metadata.json"),
            "{broken",
        )
        .await
        .unwrap();

        let sessions = store.list_sessions().await.unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].thread_id, thread_id);
        assert_eq!(sessions[0].title.as_deref(), Some("recover this session"));
        assert_eq!(sessions[0].message_count, 2);

        let repaired = fs::read(store.session_dir(&thread_id).join("metadata.json"))
            .await
            .unwrap();
        let repaired = serde_json::from_slice::<SessionMetadata>(&repaired).unwrap();
        assert_eq!(repaired.thread_id, thread_id);

        let _ = fs::remove_dir_all(base_path).await;
    }

    #[tokio::test]
    async fn load_session_recovers_malformed_metadata_and_continues() {
        let base_path = std::env::temp_dir().join(format!(
            "roder-jsonl-recover-load-test-{}",
            uuid::Uuid::new_v4()
        ));
        let store = JsonlSessionStore {
            base_path: base_path.clone(),
        };
        let thread_id = "thread-recover-load".to_string();
        let turn_id = "turn-recover-load".to_string();
        let now = OffsetDateTime::UNIX_EPOCH;

        store
            .create_session(SessionMetadata {
                thread_id: thread_id.clone(),
                title: None,
                workspace: Some("/workspace".to_string()),
                provider: Some("mock".to_string()),
                model: Some("mock".to_string()),
                runner_destination: None,
                runner_state: None,
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
                &ConversationItem::UserMessage(UserMessage::text("resume despite metadata")),
            )
            .await
            .unwrap();
        fs::write(
            store.session_dir(&thread_id).join("metadata.json"),
            "not json",
        )
        .await
        .unwrap();

        let snapshot = store.load_session(&thread_id).await.unwrap().unwrap();
        let metadata = snapshot.metadata.unwrap();

        assert_eq!(metadata.thread_id, thread_id);
        assert_eq!(metadata.title.as_deref(), Some("resume despite metadata"));
        assert_eq!(snapshot.turns.len(), 1);

        let _ = fs::remove_dir_all(base_path).await;
    }

    #[tokio::test]
    async fn load_session_accepts_valid_metadata_with_trailing_garbage_and_repairs_file() {
        let base_path = std::env::temp_dir().join(format!(
            "roder-jsonl-trailing-metadata-test-{}",
            uuid::Uuid::new_v4()
        ));
        let store = JsonlSessionStore {
            base_path: base_path.clone(),
        };
        let thread_id = "thread-trailing-metadata".to_string();
        let now = OffsetDateTime::UNIX_EPOCH;

        let metadata = SessionMetadata {
            thread_id: thread_id.clone(),
            title: Some("Recover trailing metadata".to_string()),
            workspace: Some("/workspace".to_string()),
            provider: Some("codex".to_string()),
            model: Some("gpt-5.5".to_string()),
            runner_destination: None,
            runner_state: None,
            created_at: now,
            updated_at: now,
            message_count: 1,
        };
        let dir = store.session_dir(&thread_id);
        fs::create_dir_all(&dir).await.unwrap();
        let mut corrupted = serde_json::to_string_pretty(&metadata).unwrap();
        corrupted.push('}');
        fs::write(dir.join("metadata.json"), corrupted)
            .await
            .unwrap();

        let snapshot = store.load_session(&thread_id).await.unwrap().unwrap();
        let loaded = snapshot.metadata.unwrap();

        assert_eq!(loaded.thread_id, thread_id);
        assert_eq!(loaded.title.as_deref(), Some("Recover trailing metadata"));
        assert_eq!(loaded.provider.as_deref(), Some("codex"));
        assert_eq!(loaded.model.as_deref(), Some("gpt-5.5"));
        assert_eq!(loaded.message_count, 1);

        let repaired = fs::read(dir.join("metadata.json")).await.unwrap();
        serde_json::from_slice::<SessionMetadata>(&repaired).unwrap();

        let _ = fs::remove_dir_all(base_path).await;
    }

    #[tokio::test]
    async fn append_turn_item_repairs_malformed_metadata() {
        let base_path = std::env::temp_dir().join(format!(
            "roder-jsonl-recover-append-test-{}",
            uuid::Uuid::new_v4()
        ));
        let store = JsonlSessionStore {
            base_path: base_path.clone(),
        };
        let thread_id = "thread-recover-append".to_string();
        let turn_id = "turn-recover-append".to_string();
        let now = OffsetDateTime::UNIX_EPOCH;

        store
            .create_session(SessionMetadata {
                thread_id: thread_id.clone(),
                title: None,
                workspace: None,
                provider: None,
                model: None,
                runner_destination: None,
                runner_state: None,
                created_at: now,
                updated_at: now,
                message_count: 0,
            })
            .await
            .unwrap();
        fs::write(
            store.session_dir(&thread_id).join("metadata.json"),
            "{\"thread_id\":\"thread-recover-append\"} trailing",
        )
        .await
        .unwrap();

        store
            .append_turn_item(
                &thread_id,
                &turn_id,
                &ConversationItem::UserMessage(UserMessage::text("repair me")),
            )
            .await
            .unwrap();

        let sessions = store.list_sessions().await.unwrap();

        assert_eq!(sessions[0].thread_id, thread_id);
        assert_eq!(sessions[0].title.as_deref(), Some("repair me"));
        assert_eq!(sessions[0].message_count, 1);

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
                runner_destination: None,
                runner_state: None,
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
                runner_destination: None,
                runner_state: None,
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
