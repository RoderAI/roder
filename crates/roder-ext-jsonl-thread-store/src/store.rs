use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use roder_api::events::{EventEnvelope, RoderEvent, ThreadId};
use roder_api::extension_state::ExtensionStateRecord;
use roder_api::thread::{
    ThreadItemEvent, ThreadListOptions, ThreadListPage, ThreadMetadata, ThreadSnapshot,
    ThreadStore, ThreadStoreFactory, project_turns_from_events, validate_thread_workspace,
};
use roder_api::transcript::TranscriptItem;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tokio::fs::{self, OpenOptions};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

pub struct JsonlThreadStore {
    pub base_path: PathBuf,
}

const THREAD_INDEX_FILE: &str = "thread-index.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadIndexEntry {
    thread_id: ThreadId,
    updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadIndex {
    threads: Vec<ThreadIndexEntry>,
}

impl JsonlThreadStore {
    fn thread_dir(&self, thread_id: &ThreadId) -> PathBuf {
        self.base_path.join(thread_id)
    }

    fn archived_thread_dir(&self, thread_id: &ThreadId) -> PathBuf {
        archived_threads_root(&self.base_path).join(thread_id)
    }

    fn thread_index_path(&self) -> PathBuf {
        self.base_path.join(THREAD_INDEX_FILE)
    }

    async fn load_thread_index(&self) -> anyhow::Result<Option<ThreadIndex>> {
        let path = self.thread_index_path();
        let data = match fs::read(&path).await {
            Ok(data) => data,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => {
                return Err(err).with_context(|| format!("read thread index {}", path.display()));
            }
        };
        serde_json::from_slice(&data)
            .map(Some)
            .with_context(|| format!("parse thread index {}", path.display()))
    }

    async fn write_thread_index(&self, index: &ThreadIndex) -> anyhow::Result<()> {
        let path = self.thread_index_path();
        let serialized = serde_json::to_vec_pretty(index).context("serialize thread index")?;
        fs::create_dir_all(&self.base_path)
            .await
            .with_context(|| format!("create thread root {}", self.base_path.display()))?;
        let tmp_path = path.with_file_name(format!(
            ".{}.{}.tmp",
            THREAD_INDEX_FILE,
            uuid::Uuid::new_v4()
        ));
        fs::write(&tmp_path, serialized)
            .await
            .with_context(|| format!("write thread index temp {}", tmp_path.display()))?;
        if let Err(err) = fs::rename(&tmp_path, &path).await {
            let _ = fs::remove_file(&tmp_path).await;
            return Err(err).with_context(|| format!("replace thread index {}", path.display()));
        }
        Ok(())
    }

    async fn upsert_thread_index(&self, metadata: &ThreadMetadata) -> anyhow::Result<()> {
        let mut index = self.load_thread_index().await?.unwrap_or_default();
        upsert_thread_index_entry(&mut index, metadata);
        self.write_thread_index(&index).await
    }

    async fn remove_thread_index_entry(&self, thread_id: &ThreadId) -> anyhow::Result<()> {
        let Some(mut index) = self.load_thread_index().await? else {
            return Ok(());
        };
        index.threads.retain(|entry| entry.thread_id != *thread_id);
        self.write_thread_index(&index).await
    }

    async fn rebuild_thread_index(&self) -> anyhow::Result<ThreadIndex> {
        let mut threads = self.scan_thread_index_entries().await?;
        sort_thread_index_entries(&mut threads);
        let index = ThreadIndex { threads };
        self.write_thread_index(&index).await?;
        Ok(index)
    }

    async fn scan_thread_index_entries(&self) -> anyhow::Result<Vec<ThreadIndexEntry>> {
        let mut entries = fs::read_dir(&self.base_path)
            .await
            .with_context(|| format!("read thread directory {}", self.base_path.display()))?;
        let mut threads = Vec::new();
        while let Some(entry) = entries
            .next_entry()
            .await
            .with_context(|| format!("read thread entry under {}", self.base_path.display()))?
        {
            let file_type = entry.file_type().await.with_context(|| {
                format!("read thread entry type under {}", self.base_path.display())
            })?;
            if !file_type.is_dir() {
                continue;
            }
            let thread_id = entry.file_name().to_string_lossy().to_string();
            if thread_id == "discovery-state" || is_thread_directory_without_metadata(&entry.path())
            {
                continue;
            }
            let metadata = self
                .load_or_infer_metadata(&entry.path(), &thread_id)
                .await?;
            threads.push(ThreadIndexEntry {
                thread_id,
                updated_at: metadata.updated_at,
            });
        }
        Ok(threads)
    }

    async fn load_item_events(&self, thread_id: &ThreadId) -> anyhow::Result<Vec<ThreadItemEvent>> {
        let file_path = self.thread_dir(thread_id).join("item_events.jsonl");
        if !file_path.exists() {
            return Ok(Vec::new());
        }
        let file = fs::File::open(&file_path)
            .await
            .with_context(|| format!("open item event log {}", file_path.display()))?;
        let reader = BufReader::new(file);
        let mut lines = reader.lines();
        let mut events = Vec::new();
        let mut line_number = 0usize;
        while let Some(line) = lines
            .next_line()
            .await
            .with_context(|| format!("read item event log {}", file_path.display()))?
        {
            line_number += 1;
            if line.trim().is_empty() {
                continue;
            }
            let stream = serde_json::Deserializer::from_str(&line).into_iter::<ThreadItemEvent>();
            for event in stream {
                events.push(event.with_context(|| {
                    format!(
                        "parse item event record in {}:{}",
                        file_path.display(),
                        line_number
                    )
                })?);
            }
        }
        Ok(events)
    }

    async fn write_item_event(
        &self,
        thread_id: &ThreadId,
        item_event: &ThreadItemEvent,
    ) -> anyhow::Result<()> {
        let dir = self.thread_dir(thread_id);
        fs::create_dir_all(&dir)
            .await
            .with_context(|| format!("create thread directory {}", dir.display()))?;
        let file_path = dir.join("item_events.jsonl");
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file_path)
            .await
            .with_context(|| format!("open item event log {}", file_path.display()))?;
        let mut line = serde_json::to_vec(item_event).context("serialize item event record")?;
        line.push(b'\n');
        file.write_all(&line)
            .await
            .with_context(|| format!("append item event record to {}", file_path.display()))?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl ThreadStore for JsonlThreadStore {
    fn id(&self) -> roder_api::thread::ThreadStoreId {
        "jsonl-thread-store".to_string()
    }

    fn local_thread_root(&self) -> Option<PathBuf> {
        Some(self.base_path.clone())
    }

    async fn create_thread(&self, metadata: ThreadMetadata) -> anyhow::Result<ThreadMetadata> {
        validate_thread_workspace(&metadata.workspace)?;
        let dir = self.thread_dir(&metadata.thread_id);
        fs::create_dir_all(&dir)
            .await
            .with_context(|| format!("create thread directory {}", dir.display()))?;
        let metadata_path = dir.join("metadata.json");
        write_metadata_file(&metadata_path, &metadata).await?;
        self.upsert_thread_index(&metadata).await?;
        Ok(metadata)
    }

    async fn update_thread_metadata(
        &self,
        metadata: ThreadMetadata,
    ) -> anyhow::Result<ThreadMetadata> {
        validate_thread_workspace(&metadata.workspace)?;
        let dir = self.thread_dir(&metadata.thread_id);
        fs::create_dir_all(&dir)
            .await
            .with_context(|| format!("create thread directory {}", dir.display()))?;
        let metadata_path = dir.join("metadata.json");
        write_metadata_file(&metadata_path, &metadata).await?;
        self.upsert_thread_index(&metadata).await?;
        Ok(metadata)
    }

    async fn list_threads(&self) -> anyhow::Result<Vec<ThreadMetadata>> {
        Ok(self
            .list_threads_page(ThreadListOptions::default())
            .await?
            .threads)
    }

    async fn list_threads_page(
        &self,
        options: ThreadListOptions,
    ) -> anyhow::Result<ThreadListPage> {
        if !self.base_path.exists() {
            return Ok(ThreadListPage::default());
        }
        let mut index = match self.load_thread_index().await {
            Ok(Some(index)) => index,
            Ok(None) | Err(_) => self.rebuild_thread_index().await?,
        };
        sort_thread_index_entries(&mut index.threads);

        let offset = options
            .cursor
            .as_deref()
            .and_then(|cursor| cursor.parse::<usize>().ok())
            .unwrap_or(0)
            .min(index.threads.len());
        let limit = options
            .limit
            .unwrap_or(index.threads.len().saturating_sub(offset));
        let next_offset = offset.saturating_add(limit).min(index.threads.len());
        let mut threads = Vec::new();
        let mut stale = false;
        for entry in index.threads.iter().skip(offset).take(limit) {
            match self.load_thread_metadata(&entry.thread_id).await? {
                Some(metadata) => threads.push(metadata),
                None => stale = true,
            }
        }
        if stale || threads.len() < limit.min(index.threads.len().saturating_sub(offset)) {
            index = self.rebuild_thread_index().await?;
            threads.clear();
            for entry in index.threads.iter().skip(offset).take(limit) {
                if let Some(metadata) = self.load_thread_metadata(&entry.thread_id).await? {
                    threads.push(metadata);
                }
            }
        }
        threads.sort_by_key(|thread| std::cmp::Reverse(thread.updated_at));
        Ok(ThreadListPage {
            threads,
            next_cursor: (next_offset < index.threads.len()).then(|| next_offset.to_string()),
            backwards_cursor: (offset > 0).then(|| offset.saturating_sub(limit).to_string()),
        })
    }

    async fn load_thread(&self, thread_id: &ThreadId) -> anyhow::Result<Option<ThreadSnapshot>> {
        let dir = self.thread_dir(thread_id);
        if !dir.exists() {
            return Ok(None);
        }
        let events = if dir.join("metadata.json").exists() {
            None
        } else {
            match self.load_events(thread_id).await {
                Ok(events) => Some(events),
                Err(_) => return Ok(None),
            }
        };
        let metadata = Some(self.load_or_infer_metadata(&dir, thread_id).await?);
        let events = match events {
            Some(events) => events,
            None => self.load_events(thread_id).await?,
        };
        let turns = project_turns_from_events(thread_id, &events);
        let item_events = self.load_item_events(thread_id).await?;
        let extension_states = self.load_extension_states(thread_id).await?;
        Ok(Some(ThreadSnapshot {
            metadata,
            events,
            turns,
            item_events,
            extension_states,
        }))
    }

    async fn load_thread_metadata(
        &self,
        thread_id: &ThreadId,
    ) -> anyhow::Result<Option<ThreadMetadata>> {
        let dir = self.thread_dir(thread_id);
        if !dir.exists() {
            return Ok(None);
        }
        if !dir.join("metadata.json").exists() && self.load_events(thread_id).await.is_err() {
            return Ok(None);
        }
        Ok(Some(self.load_or_infer_metadata(&dir, thread_id).await?))
    }

    async fn archive_thread(&self, thread_id: &ThreadId) -> anyhow::Result<bool> {
        let source = self.thread_dir(thread_id);
        if !source.exists() {
            return Ok(false);
        }
        let archive_root = archived_threads_root(&self.base_path);
        fs::create_dir_all(&archive_root).await.with_context(|| {
            format!(
                "create archived threads directory {}",
                archive_root.display()
            )
        })?;
        let destination = self.archived_thread_dir(thread_id);
        if destination.exists() {
            anyhow::bail!("archived thread already exists: {}", destination.display());
        }
        fs::rename(&source, &destination).await.with_context(|| {
            format!(
                "archive thread {} from {} to {}",
                thread_id,
                source.display(),
                destination.display()
            )
        })?;
        self.remove_thread_index_entry(thread_id).await?;
        Ok(true)
    }

    async fn append_event(
        &self,
        thread_id: &ThreadId,
        envelope: &EventEnvelope,
    ) -> anyhow::Result<()> {
        let dir = self.thread_dir(thread_id);
        fs::create_dir_all(&dir)
            .await
            .with_context(|| format!("create thread directory {}", dir.display()))?;
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
        if let RoderEvent::TranscriptItemAppended(event) = &envelope.event
            && let Some(item) = &event.item
        {
            self.update_metadata_for_turn_item(thread_id, item, envelope.timestamp)
                .await?;
        }
        Ok(())
    }

    async fn append_item_event(
        &self,
        thread_id: &ThreadId,
        item_event: &ThreadItemEvent,
    ) -> anyhow::Result<()> {
        self.write_item_event(thread_id, item_event).await
    }

    async fn append_extension_state(
        &self,
        thread_id: &ThreadId,
        record: &ExtensionStateRecord,
    ) -> anyhow::Result<()> {
        let dir = self.thread_dir(thread_id);
        fs::create_dir_all(&dir)
            .await
            .with_context(|| format!("create thread directory {}", dir.display()))?;
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

impl JsonlThreadStore {
    async fn load_extension_states(
        &self,
        thread_id: &ThreadId,
    ) -> anyhow::Result<Vec<ExtensionStateRecord>> {
        let file_path = self.thread_dir(thread_id).join("extension_state.jsonl");
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
        let file_path = self.thread_dir(thread_id).join("events.jsonl");
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
        item: &TranscriptItem,
        event_timestamp: OffsetDateTime,
    ) -> anyhow::Result<()> {
        let metadata_path = self.thread_dir(thread_id).join("metadata.json");
        let (mut metadata, count_current_item) = if metadata_path.exists() {
            let data = fs::read(&metadata_path)
                .await
                .with_context(|| format!("read thread metadata {}", metadata_path.display()))?;
            match parse_metadata_tolerant(&data) {
                Ok((metadata, _needs_repair)) => (self.with_derived_title(metadata).await?, true),
                Err(err) => {
                    anyhow::bail!(
                        "thread metadata invalid for {}: {err}",
                        metadata_path.display()
                    )
                }
            }
        } else {
            (
                self.infer_metadata_from_thread_dir(
                    &self.thread_dir(thread_id),
                    thread_id,
                    None,
                    event_timestamp,
                )
                .await?,
                false,
            )
        };
        metadata.updated_at = OffsetDateTime::now_utc();
        if count_current_item
            && matches!(
                item,
                TranscriptItem::UserMessage(_) | TranscriptItem::AssistantMessage(_)
            )
        {
            metadata.message_count = metadata.message_count.saturating_add(1);
        }
        if metadata
            .title
            .as_ref()
            .is_none_or(|title| title.trim().is_empty())
            && let TranscriptItem::UserMessage(message) = item
        {
            metadata.title = title_from_user_text(&message.text);
        }
        write_metadata_file(&metadata_path, &metadata).await?;
        self.upsert_thread_index(&metadata).await?;
        Ok(())
    }

    async fn load_or_infer_metadata(
        &self,
        dir: &Path,
        thread_id: &ThreadId,
    ) -> anyhow::Result<ThreadMetadata> {
        let metadata_path = dir.join("metadata.json");
        if metadata_path.exists() {
            let data = fs::read(&metadata_path)
                .await
                .with_context(|| format!("read thread metadata {}", metadata_path.display()))?;
            match parse_metadata_tolerant(&data) {
                Ok((metadata, needs_repair)) => {
                    let metadata = self.with_derived_title(metadata).await?;
                    if needs_repair {
                        self.repair_metadata_file(&metadata_path, &metadata).await;
                    }
                    return Ok(metadata);
                }
                Err(_) => {
                    anyhow::bail!("thread metadata invalid for {}", metadata_path.display());
                }
            }
        }

        let metadata = self
            .infer_metadata_from_thread_dir(dir, thread_id, None, OffsetDateTime::now_utc())
            .await?;
        self.repair_metadata_file(&metadata_path, &metadata).await;
        Ok(metadata)
    }

    async fn infer_metadata_from_thread_dir(
        &self,
        dir: &Path,
        thread_id: &ThreadId,
        current_item: Option<&TranscriptItem>,
        fallback_timestamp: OffsetDateTime,
    ) -> anyhow::Result<ThreadMetadata> {
        let events = self.load_events(thread_id).await.unwrap_or_default();
        let turns = project_turns_from_events(thread_id, &events);
        let mut first_timestamp = events
            .first()
            .map(|event| event.timestamp)
            .unwrap_or(fallback_timestamp);
        let mut last_timestamp = events
            .last()
            .map(|event| event.timestamp)
            .unwrap_or(fallback_timestamp);
        let mut message_count = 0u32;
        let mut title = None;

        for turn in &turns {
            if turn.created_at < first_timestamp {
                first_timestamp = turn.created_at;
            }
            if let Some(completed_at) = turn.completed_at
                && completed_at > last_timestamp
            {
                last_timestamp = completed_at;
            }
            for item in &turn.items {
                if matches!(
                    item,
                    TranscriptItem::UserMessage(_) | TranscriptItem::AssistantMessage(_)
                ) {
                    message_count = message_count.saturating_add(1);
                }
                if title.is_none()
                    && let TranscriptItem::UserMessage(message) = item
                {
                    title = title_from_user_text(&message.text);
                }
            }
        }

        if let Some(item) = current_item {
            if matches!(
                item,
                TranscriptItem::UserMessage(_) | TranscriptItem::AssistantMessage(_)
            ) {
                message_count = message_count.saturating_add(1);
            }
            if title.is_none()
                && let TranscriptItem::UserMessage(message) = item
            {
                title = title_from_user_text(&message.text);
            }
        }

        Ok(ThreadMetadata {
            thread_id: thread_id.clone(),
            title,
            workspace: infer_workspace_for_recovered_thread(dir),
            workspace_id: None,
            root_id: None,
            provider: None,
            model: None,
            selection_mode: None,
            tool_allowlist: Vec::new(),
            developer_instructions: None,
            external_tools: Vec::new(),
            runner_destination: None,
            runner_state: None,
            runner_binding: None,
            created_at: first_timestamp,
            updated_at: last_timestamp,
            message_count,
            usage: None,
        })
    }

    async fn repair_metadata_file(&self, metadata_path: &Path, metadata: &ThreadMetadata) {
        let _ = write_metadata_file(metadata_path, metadata).await;
    }

    async fn with_derived_title(
        &self,
        mut metadata: ThreadMetadata,
    ) -> anyhow::Result<ThreadMetadata> {
        if metadata
            .title
            .as_ref()
            .is_some_and(|title| !title.trim().is_empty())
        {
            return Ok(metadata);
        }
        let Ok(events) = self.load_events(&metadata.thread_id).await else {
            return Ok(metadata);
        };
        let turns = project_turns_from_events(&metadata.thread_id, &events);
        for turn in turns {
            for item in turn.items {
                if let TranscriptItem::UserMessage(message) = item
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

async fn write_metadata_file(
    metadata_path: &Path,
    metadata: &ThreadMetadata,
) -> anyhow::Result<()> {
    let serialized = serde_json::to_vec_pretty(metadata).context("serialize thread metadata")?;
    let parent = metadata_path
        .parent()
        .with_context(|| format!("metadata path has no parent: {}", metadata_path.display()))?;
    fs::create_dir_all(parent)
        .await
        .with_context(|| format!("create thread directory {}", parent.display()))?;
    let file_name = metadata_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("metadata.json");
    let tmp_path =
        metadata_path.with_file_name(format!(".{file_name}.{}.tmp", uuid::Uuid::new_v4()));
    fs::write(&tmp_path, serialized)
        .await
        .with_context(|| format!("write thread metadata temp {}", tmp_path.display()))?;
    if let Err(err) = fs::rename(&tmp_path, metadata_path).await {
        let _ = fs::remove_file(&tmp_path).await;
        return Err(err).with_context(|| {
            format!(
                "replace thread metadata {} with {}",
                metadata_path.display(),
                tmp_path.display()
            )
        });
    }
    Ok(())
}

fn parse_metadata_tolerant(data: &[u8]) -> serde_json::Result<(ThreadMetadata, bool)> {
    match serde_json::from_slice::<ThreadMetadata>(data) {
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

fn archived_threads_root(active_threads_root: &Path) -> PathBuf {
    active_threads_root
        .parent()
        .map(|parent| parent.join("archived_threads"))
        .unwrap_or_else(|| active_threads_root.with_file_name("archived_threads"))
}

fn sort_thread_index_entries(entries: &mut [ThreadIndexEntry]) {
    entries.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| right.thread_id.cmp(&left.thread_id))
    });
}

fn upsert_thread_index_entry(index: &mut ThreadIndex, metadata: &ThreadMetadata) {
    if let Some(entry) = index
        .threads
        .iter_mut()
        .find(|entry| entry.thread_id == metadata.thread_id)
    {
        entry.updated_at = metadata.updated_at;
    } else {
        index.threads.push(ThreadIndexEntry {
            thread_id: metadata.thread_id.clone(),
            updated_at: metadata.updated_at,
        });
    }
    sort_thread_index_entries(&mut index.threads);
}

fn is_thread_directory_without_metadata(dir: &Path) -> bool {
    !dir.join("metadata.json").exists()
}

fn infer_workspace_for_recovered_thread(dir: &Path) -> String {
    std::env::current_dir()
        .unwrap_or_else(|_| dir.to_path_buf())
        .display()
        .to_string()
}

pub struct JsonlThreadStoreFactory {
    pub base_path: PathBuf,
}

impl ThreadStoreFactory for JsonlThreadStoreFactory {
    fn id(&self) -> roder_api::thread::ThreadStoreId {
        "jsonl-thread-store".to_string()
    }

    fn create(&self) -> Arc<dyn ThreadStore> {
        Arc::new(JsonlThreadStore {
            base_path: self.base_path.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::events::{EventSource, SubagentTraceCreated, TurnCompleted};
    use roder_api::inference::InferenceEvent;
    use roder_api::thread::{ThreadItemDelta, ThreadItemEventKind};
    use roder_api::trace::{
        ParentTurnRef, SubagentDestination, SubagentDestinationKind, SubagentTraceStatus,
        SubagentTraceSummary,
    };
    use roder_api::transcript::{AssistantMessage, TranscriptItem, UserMessage};

    fn test_workspace(name: &str) -> String {
        std::env::temp_dir().join(name).display().to_string()
    }

    fn transcript_item_event(
        seq: u64,
        thread_id: &ThreadId,
        turn_id: &str,
        item: TranscriptItem,
        timestamp: OffsetDateTime,
    ) -> EventEnvelope {
        let item_type = match &item {
            TranscriptItem::UserMessage(_) => "user_message",
            TranscriptItem::AssistantMessage(_) => "assistant_message",
            TranscriptItem::ReasoningSummary(_) => "reasoning_summary",
            TranscriptItem::ToolCall(_) => "tool_call",
            TranscriptItem::ToolResult(_) => "tool_result",
            TranscriptItem::FileChange(_) => "file_change",
            TranscriptItem::ContextCompaction(_) => "context_compaction",
            TranscriptItem::Error(_) => "error",
            TranscriptItem::ProviderMetadata(_) => "provider_metadata",
        };
        EventEnvelope {
            event_id: format!("transcript-event-{seq}"),
            seq,
            timestamp,
            source: EventSource::Core,
            kind: "transcript.item_appended".to_string(),
            thread_id: Some(thread_id.clone()),
            turn_id: Some(turn_id.to_string()),
            event: RoderEvent::TranscriptItemAppended(roder_api::events::TranscriptItemAppended {
                thread_id: thread_id.clone(),
                turn_id: turn_id.to_string(),
                item_type: item_type.to_string(),
                item_index: Some(seq.saturating_sub(1) as usize),
                item: Some(item),
                timestamp,
            }),
        }
    }

    async fn append_test_turn_item(
        store: &JsonlThreadStore,
        thread_id: &ThreadId,
        turn_id: &str,
        seq: u64,
        item: TranscriptItem,
        timestamp: OffsetDateTime,
    ) {
        store
            .append_event(
                thread_id,
                &transcript_item_event(seq, thread_id, turn_id, item, timestamp),
            )
            .await
            .unwrap();
    }

    async fn write_event_without_metadata(
        store: &JsonlThreadStore,
        thread_id: &ThreadId,
        envelope: &EventEnvelope,
    ) {
        let dir = store.thread_dir(thread_id);
        fs::create_dir_all(&dir).await.unwrap();
        let file_path = dir.join("events.jsonl");
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file_path)
            .await
            .unwrap();
        let mut line = serde_json::to_vec(envelope).unwrap();
        line.push(b'\n');
        file.write_all(&line).await.unwrap();
    }

    #[tokio::test]
    async fn load_thread_projects_turn_items_and_completion() {
        let base_path = std::env::temp_dir().join(format!(
            "roder-jsonl-thread-store-test-{}",
            uuid::Uuid::new_v4()
        ));
        let store = JsonlThreadStore {
            base_path: base_path.clone(),
        };
        let thread_id = "thread-a".to_string();
        let turn_id = "turn-a".to_string();
        let now = OffsetDateTime::UNIX_EPOCH;

        store
            .create_thread(ThreadMetadata {
                thread_id: thread_id.clone(),
                title: Some("Resume me".to_string()),
                workspace: test_workspace("workspace"),
                workspace_id: None,
                root_id: None,
                provider: Some("mock".to_string()),
                model: Some("mock".to_string()),
                selection_mode: None,
                tool_allowlist: Vec::new(),
                developer_instructions: None,
                external_tools: Vec::new(),
                runner_destination: None,
                runner_state: None,
                runner_binding: None,
                created_at: now,
                updated_at: now,
                message_count: 0,
                usage: None,
            })
            .await
            .unwrap();
        append_test_turn_item(
            &store,
            &thread_id,
            &turn_id,
            1,
            TranscriptItem::UserMessage(UserMessage::text("hello")),
            now,
        )
        .await;
        append_test_turn_item(
            &store,
            &thread_id,
            &turn_id,
            2,
            TranscriptItem::AssistantMessage(AssistantMessage {
                text: "world".to_string(),
                phase: None,
            }),
            now,
        )
        .await;
        store
            .append_event(
                &thread_id,
                &EventEnvelope {
                    event_id: "event-a".to_string(),
                    seq: 3,
                    timestamp: now,
                    source: EventSource::Core,
                    kind: "turn.completed".to_string(),
                    thread_id: Some(thread_id.clone()),
                    turn_id: Some(turn_id.clone()),
                    event: RoderEvent::TurnCompleted(TurnCompleted {
                        thread_id: thread_id.clone(),
                        turn_id: turn_id.clone(),
                        usage: None,
                        finish_reason: None,
                        timestamp: now,
                    }),
                },
            )
            .await
            .unwrap();

        let snapshot = store.load_thread(&thread_id).await.unwrap().unwrap();

        assert_eq!(
            snapshot.metadata.unwrap().title.as_deref(),
            Some("Resume me")
        );
        assert_eq!(snapshot.events.len(), 3);
        assert_eq!(snapshot.turns.len(), 1);
        assert_eq!(snapshot.turns[0].turn_id, turn_id);
        assert_eq!(snapshot.turns[0].items.len(), 2);
        assert_eq!(snapshot.turns[0].completed_at, Some(now));

        let _ = fs::remove_dir_all(base_path).await;
    }

    #[tokio::test]
    async fn load_thread_recovers_missing_metadata_from_events() {
        let base_path = std::env::temp_dir().join(format!(
            "roder-jsonl-missing-metadata-load-test-{}",
            uuid::Uuid::new_v4()
        ));
        let store = JsonlThreadStore {
            base_path: base_path.clone(),
        };
        let thread_id = "thread-missing-metadata".to_string();
        let turn_id = "turn-a".to_string();
        let now = OffsetDateTime::UNIX_EPOCH;

        write_event_without_metadata(
            &store,
            &thread_id,
            &transcript_item_event(
                1,
                &thread_id,
                &turn_id,
                TranscriptItem::UserMessage(UserMessage::text("recover my title")),
                now,
            ),
        )
        .await;

        let snapshot = store.load_thread(&thread_id).await.unwrap().unwrap();
        let metadata = snapshot.metadata.unwrap();

        assert_eq!(metadata.thread_id, thread_id);
        assert_eq!(metadata.title.as_deref(), Some("recover my title"));
        assert_eq!(metadata.message_count, 1);
        assert!(base_path.join(&thread_id).join("metadata.json").exists());
        assert_eq!(snapshot.turns.len(), 1);

        let _ = fs::remove_dir_all(base_path).await;
    }

    #[tokio::test]
    async fn append_event_repairs_missing_metadata_instead_of_failing() {
        let base_path = std::env::temp_dir().join(format!(
            "roder-jsonl-missing-metadata-append-test-{}",
            uuid::Uuid::new_v4()
        ));
        let store = JsonlThreadStore {
            base_path: base_path.clone(),
        };
        let thread_id = "thread-append-recovers-metadata".to_string();
        let turn_id = "turn-a".to_string();
        let now = OffsetDateTime::UNIX_EPOCH;

        write_event_without_metadata(
            &store,
            &thread_id,
            &transcript_item_event(
                1,
                &thread_id,
                &turn_id,
                TranscriptItem::UserMessage(UserMessage::text("old event title")),
                now,
            ),
        )
        .await;

        store
            .append_event(
                &thread_id,
                &transcript_item_event(
                    2,
                    &thread_id,
                    &turn_id,
                    TranscriptItem::AssistantMessage(AssistantMessage {
                        text: "second message".to_string(),
                        phase: None,
                    }),
                    now,
                ),
            )
            .await
            .unwrap();

        let snapshot = store.load_thread(&thread_id).await.unwrap().unwrap();
        let metadata = snapshot.metadata.unwrap();

        assert_eq!(metadata.title.as_deref(), Some("old event title"));
        assert_eq!(metadata.message_count, 2);
        assert_eq!(snapshot.events.len(), 2);

        let _ = fs::remove_dir_all(base_path).await;
    }

    #[tokio::test]
    async fn append_event_keeps_raw_events_separate_from_public_item_events() {
        let base_path = std::env::temp_dir().join(format!(
            "roder-jsonl-item-event-seq-test-{}",
            uuid::Uuid::new_v4()
        ));
        let store = JsonlThreadStore {
            base_path: base_path.clone(),
        };
        let thread_id = "thread-item-seq".to_string();
        let turn_id = "turn-item-seq".to_string();
        let now = OffsetDateTime::UNIX_EPOCH;

        store
            .create_thread(ThreadMetadata {
                thread_id: thread_id.clone(),
                title: Some("Keep item seq".to_string()),
                workspace: test_workspace("workspace"),
                workspace_id: None,
                root_id: None,
                provider: Some("mock".to_string()),
                model: Some("mock".to_string()),
                selection_mode: None,
                tool_allowlist: Vec::new(),
                developer_instructions: None,
                external_tools: Vec::new(),
                runner_destination: None,
                runner_state: None,
                runner_binding: None,
                created_at: now,
                updated_at: now,
                message_count: 0,
                usage: None,
            })
            .await
            .unwrap();
        store
            .append_event(
                &thread_id,
                &EventEnvelope {
                    event_id: "raw-reasoning-event".to_string(),
                    seq: 1,
                    timestamp: now,
                    source: EventSource::Core,
                    kind: "inference.event_received".to_string(),
                    thread_id: Some(thread_id.clone()),
                    turn_id: Some(turn_id.clone()),
                    event: RoderEvent::InferenceEventReceived(
                        roder_api::events::InferenceEventReceived {
                            thread_id: thread_id.clone(),
                            turn_id: turn_id.clone(),
                            event: InferenceEvent::ReasoningDelta(
                                roder_api::inference::ReasoningDelta {
                                    text: "thinking".to_string(),
                                },
                            ),
                            timestamp: now,
                        },
                    ),
                },
            )
            .await
            .unwrap();

        let item_events = store.load_item_events(&thread_id).await.unwrap();
        assert!(item_events.is_empty());

        store
            .append_item_event(
                &thread_id,
                &ThreadItemEvent {
                    seq: 1,
                    event_id: "item-event-1".to_string(),
                    thread_id: thread_id.clone(),
                    turn_id: turn_id.clone(),
                    timestamp: now,
                    event: ThreadItemEventKind::ItemDelta {
                        item_id: "turn-item-seq-agent-reasoning".to_string(),
                        delta: ThreadItemDelta::ReasoningText {
                            delta: "thinking".to_string(),
                            content_index: 0,
                        },
                    },
                },
            )
            .await
            .unwrap();

        let item_events = store.load_item_events(&thread_id).await.unwrap();
        assert_eq!(item_events.len(), 1);
        assert_eq!(item_events[0].seq, 1);
        assert_eq!(item_events[0].event_id, "item-event-1");
        assert!(store.thread_dir(&thread_id).join("events.jsonl").exists());
        assert!(
            store
                .thread_dir(&thread_id)
                .join("item_events.jsonl")
                .exists()
        );
        assert!(
            !store
                .thread_dir(&thread_id)
                .join("transcript_items.jsonl")
                .exists()
        );

        let _ = fs::remove_dir_all(base_path).await;
    }

    #[tokio::test]
    async fn archive_thread_moves_thread_out_of_active_list() {
        let base_path =
            std::env::temp_dir().join(format!("roder-jsonl-archive-test-{}", uuid::Uuid::new_v4()));
        let store = JsonlThreadStore {
            base_path: base_path.clone(),
        };
        let thread_id = "thread-archive".to_string();
        let now = OffsetDateTime::UNIX_EPOCH;

        store
            .create_thread(ThreadMetadata {
                thread_id: thread_id.clone(),
                title: Some("Archive me".to_string()),
                workspace: test_workspace("workspace"),
                workspace_id: None,
                root_id: None,
                provider: Some("mock".to_string()),
                model: Some("mock".to_string()),
                selection_mode: None,
                tool_allowlist: Vec::new(),
                developer_instructions: None,
                external_tools: Vec::new(),
                runner_destination: None,
                runner_state: None,
                runner_binding: None,
                created_at: now,
                updated_at: now,
                message_count: 0,
                usage: None,
            })
            .await
            .unwrap();

        assert!(store.archive_thread(&thread_id).await.unwrap());
        assert!(store.list_threads().await.unwrap().is_empty());
        assert!(store.load_thread(&thread_id).await.unwrap().is_none());
        assert!(
            base_path
                .parent()
                .unwrap()
                .join("archived_threads")
                .join(&thread_id)
                .join("metadata.json")
                .exists()
        );

        let _ = fs::remove_dir_all(base_path.parent().unwrap().join("archived_threads")).await;
        let _ = fs::remove_dir_all(base_path).await;
    }

    #[tokio::test]
    async fn list_threads_skips_runtime_event_directory_without_metadata() {
        let base_path = std::env::temp_dir().join(format!(
            "roder-jsonl-runtime-sentinel-test-{}",
            uuid::Uuid::new_v4()
        ));
        let store = JsonlThreadStore {
            base_path: base_path.clone(),
        };
        let runtime_dir = store.thread_dir(&"runtime".to_string());
        fs::create_dir_all(&runtime_dir).await.unwrap();
        fs::write(runtime_dir.join("events.jsonl"), "{}\n")
            .await
            .unwrap();

        assert!(store.list_threads().await.unwrap().is_empty());

        let _ = fs::remove_dir_all(base_path).await;
    }

    #[tokio::test]
    async fn app_server_artifact_directory_without_metadata_is_not_a_thread() {
        let base_path = std::env::temp_dir().join(format!(
            "roder-jsonl-app-server-sentinel-test-{}",
            uuid::Uuid::new_v4()
        ));
        let store = JsonlThreadStore {
            base_path: base_path.clone(),
        };
        let app_server_dir = store.thread_dir(&"app-server".to_string());
        fs::create_dir_all(app_server_dir.join("artifacts").join("command-1"))
            .await
            .unwrap();
        fs::write(app_server_dir.join("events.jsonl"), "{}\n")
            .await
            .unwrap();

        assert!(store.list_threads().await.unwrap().is_empty());
        assert!(
            store
                .load_thread(&"app-server".to_string())
                .await
                .unwrap()
                .is_none()
        );

        let _ = fs::remove_dir_all(base_path).await;
    }

    #[tokio::test]
    async fn workflow_event_directory_without_metadata_is_not_a_thread() {
        let base_path = std::env::temp_dir().join(format!(
            "roder-jsonl-workflow-sentinel-test-{}",
            uuid::Uuid::new_v4()
        ));
        let store = JsonlThreadStore {
            base_path: base_path.clone(),
        };
        let synthetic_dir = store.thread_dir(&"thread-workflow".to_string());
        fs::create_dir_all(&synthetic_dir).await.unwrap();
        fs::write(synthetic_dir.join("events.jsonl"), "{}\n")
            .await
            .unwrap();

        assert!(store.list_threads().await.unwrap().is_empty());
        assert!(
            store
                .load_thread(&"thread-workflow".to_string())
                .await
                .unwrap()
                .is_none()
        );

        let _ = fs::remove_dir_all(base_path).await;
    }

    #[tokio::test]
    async fn caller_named_event_directory_without_metadata_is_not_a_thread() {
        let base_path = std::env::temp_dir().join(format!(
            "roder-jsonl-caller-named-event-dir-test-{}",
            uuid::Uuid::new_v4()
        ));
        let store = JsonlThreadStore {
            base_path: base_path.clone(),
        };
        let event_dir = store.thread_dir(&"thread-discovery".to_string());
        fs::create_dir_all(&event_dir).await.unwrap();
        fs::write(event_dir.join("events.jsonl"), "{}\n")
            .await
            .unwrap();

        assert!(store.list_threads().await.unwrap().is_empty());
        assert!(
            store
                .load_thread(&"thread-discovery".to_string())
                .await
                .unwrap()
                .is_none()
        );

        let _ = fs::remove_dir_all(base_path).await;
    }

    #[tokio::test]
    async fn caller_named_thread_with_metadata_is_a_thread() {
        let base_path = std::env::temp_dir().join(format!(
            "roder-jsonl-caller-named-thread-test-{}",
            uuid::Uuid::new_v4()
        ));
        let store = JsonlThreadStore {
            base_path: base_path.clone(),
        };
        let thread_id = "thread-discovery".to_string();
        let now = OffsetDateTime::UNIX_EPOCH;
        store
            .create_thread(ThreadMetadata {
                thread_id: thread_id.clone(),
                title: Some("Discovery".to_string()),
                workspace: test_workspace("workspace"),
                workspace_id: None,
                root_id: None,
                provider: Some("mock".to_string()),
                model: Some("mock".to_string()),
                selection_mode: None,
                tool_allowlist: Vec::new(),
                developer_instructions: None,
                external_tools: Vec::new(),
                runner_destination: None,
                runner_state: None,
                runner_binding: None,
                created_at: now,
                updated_at: now,
                message_count: 0,
                usage: None,
            })
            .await
            .unwrap();

        assert_eq!(store.list_threads().await.unwrap()[0].thread_id, thread_id);
        assert!(
            store
                .load_thread(&"thread-discovery".to_string())
                .await
                .unwrap()
                .is_some()
        );

        let _ = fs::remove_dir_all(base_path).await;
    }

    #[tokio::test]
    async fn extension_state_round_trips_through_thread_snapshot() {
        let base_path = std::env::temp_dir().join(format!(
            "roder-jsonl-extension-state-{}",
            uuid::Uuid::new_v4()
        ));
        let store = JsonlThreadStore {
            base_path: base_path.clone(),
        };
        let thread_id = "thread-state".to_string();
        let now = OffsetDateTime::UNIX_EPOCH;

        store
            .create_thread(ThreadMetadata {
                thread_id: thread_id.clone(),
                title: None,
                workspace: test_workspace("workspace"),
                workspace_id: None,
                root_id: None,
                provider: None,
                model: None,
                selection_mode: None,
                tool_allowlist: Vec::new(),
                developer_instructions: None,
                external_tools: Vec::new(),
                runner_destination: None,
                runner_state: None,
                runner_binding: None,
                created_at: now,
                updated_at: now,
                message_count: 0,
                usage: None,
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

        let snapshot = store.load_thread(&thread_id).await.unwrap().unwrap();

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
        let store = JsonlThreadStore {
            base_path: base_path.clone(),
        };
        let thread_id = "parent-thread".to_string();
        let turn_id = "parent-turn".to_string();
        let now = OffsetDateTime::UNIX_EPOCH;

        store
            .create_thread(ThreadMetadata {
                thread_id: thread_id.clone(),
                title: Some("Trace me".to_string()),
                workspace: test_workspace("workspace"),
                workspace_id: None,
                root_id: None,
                provider: Some("mock".to_string()),
                model: Some("mock".to_string()),
                selection_mode: None,
                tool_allowlist: Vec::new(),
                developer_instructions: None,
                external_tools: Vec::new(),
                runner_destination: None,
                runner_state: None,
                runner_binding: None,
                created_at: now,
                updated_at: now,
                message_count: 0,
                usage: None,
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

        let snapshot = store.load_thread(&thread_id).await.unwrap().unwrap();

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
    async fn load_thread_preserves_provider_metadata_with_encrypted_reasoning() {
        let base_path = std::env::temp_dir().join(format!(
            "roder-jsonl-thread-store-test-{}",
            uuid::Uuid::new_v4()
        ));
        let store = JsonlThreadStore {
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
            .create_thread(ThreadMetadata {
                thread_id: thread_id.clone(),
                title: Some("Resume encrypted reasoning".to_string()),
                workspace: test_workspace("workspace"),
                workspace_id: None,
                root_id: None,
                provider: Some("openai".to_string()),
                model: Some("gpt-5.5".to_string()),
                selection_mode: None,
                tool_allowlist: Vec::new(),
                developer_instructions: None,
                external_tools: Vec::new(),
                runner_destination: None,
                runner_state: None,
                runner_binding: None,
                created_at: now,
                updated_at: now,
                message_count: 0,
                usage: None,
            })
            .await
            .unwrap();
        append_test_turn_item(
            &store,
            &thread_id,
            &turn_id,
            1,
            TranscriptItem::ProviderMetadata(metadata.clone()),
            now,
        )
        .await;

        let snapshot = store.load_thread(&thread_id).await.unwrap().unwrap();

        assert_eq!(
            snapshot.turns[0].items[0],
            TranscriptItem::ProviderMetadata(metadata)
        );

        let _ = fs::remove_dir_all(base_path).await;
    }

    #[tokio::test]
    async fn append_event_updates_metadata_counts_and_recency() {
        let base_path = std::env::temp_dir().join(format!(
            "roder-jsonl-metadata-test-{}",
            uuid::Uuid::new_v4()
        ));
        let store = JsonlThreadStore {
            base_path: base_path.clone(),
        };
        let thread_id = "thread-metadata".to_string();
        let turn_id = "turn-metadata".to_string();
        let now = OffsetDateTime::UNIX_EPOCH;

        store
            .create_thread(ThreadMetadata {
                thread_id: thread_id.clone(),
                title: Some("Metadata".to_string()),
                workspace: test_workspace("workspace"),
                workspace_id: None,
                root_id: None,
                provider: Some("mock".to_string()),
                model: Some("mock".to_string()),
                selection_mode: None,
                tool_allowlist: Vec::new(),
                developer_instructions: None,
                external_tools: Vec::new(),
                runner_destination: None,
                runner_state: None,
                runner_binding: None,
                created_at: now,
                updated_at: now,
                message_count: 0,
                usage: None,
            })
            .await
            .unwrap();
        append_test_turn_item(
            &store,
            &thread_id,
            &turn_id,
            1,
            TranscriptItem::UserMessage(UserMessage::text("hello")),
            now,
        )
        .await;
        append_test_turn_item(
            &store,
            &thread_id,
            &turn_id,
            2,
            TranscriptItem::ProviderMetadata(serde_json::json!({"id": "resp_1"})),
            now,
        )
        .await;
        append_test_turn_item(
            &store,
            &thread_id,
            &turn_id,
            3,
            TranscriptItem::AssistantMessage(AssistantMessage {
                text: "world".to_string(),
                phase: None,
            }),
            now,
        )
        .await;

        let threads = store.list_threads().await.unwrap();

        assert_eq!(threads[0].message_count, 2);
        assert!(threads[0].updated_at > now);

        let _ = fs::remove_dir_all(base_path).await;
    }

    #[tokio::test]
    async fn list_threads_page_returns_newest_metadata_page() {
        let base_path =
            std::env::temp_dir().join(format!("roder-jsonl-page-test-{}", uuid::Uuid::new_v4()));
        let store = JsonlThreadStore {
            base_path: base_path.clone(),
        };

        for index in 0..3 {
            let timestamp = OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(index);
            store
                .create_thread(ThreadMetadata {
                    thread_id: format!("thread-{index}"),
                    title: Some(format!("Thread {index}")),
                    workspace: test_workspace("workspace"),
                    workspace_id: None,
                    root_id: None,
                    provider: Some("mock".to_string()),
                    model: Some("mock".to_string()),
                    selection_mode: None,
                    tool_allowlist: Vec::new(),
                    developer_instructions: None,
                    external_tools: Vec::new(),
                    runner_destination: None,
                    runner_state: None,
                    runner_binding: None,
                    created_at: timestamp,
                    updated_at: timestamp,
                    message_count: index as u32,
                    usage: None,
                })
                .await
                .unwrap();
        }

        let first_page = store
            .list_threads_page(ThreadListOptions {
                limit: Some(2),
                cursor: None,
            })
            .await
            .unwrap();
        assert_eq!(first_page.threads.len(), 2);
        assert_eq!(first_page.threads[0].thread_id, "thread-2");
        assert_eq!(first_page.threads[1].thread_id, "thread-1");
        assert_eq!(first_page.next_cursor.as_deref(), Some("2"));
        assert!(base_path.join(THREAD_INDEX_FILE).exists());

        let second_page = store
            .list_threads_page(ThreadListOptions {
                limit: Some(2),
                cursor: first_page.next_cursor,
            })
            .await
            .unwrap();
        assert_eq!(second_page.threads.len(), 1);
        assert_eq!(second_page.threads[0].thread_id, "thread-0");
        assert!(second_page.next_cursor.is_none());

        let _ = fs::remove_dir_all(base_path).await;
    }

    #[tokio::test]
    async fn list_threads_page_rebuilds_stale_thread_index() {
        let base_path = std::env::temp_dir().join(format!(
            "roder-jsonl-stale-page-test-{}",
            uuid::Uuid::new_v4()
        ));
        let store = JsonlThreadStore {
            base_path: base_path.clone(),
        };

        for index in 0..2 {
            let timestamp = OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(index);
            store
                .create_thread(ThreadMetadata {
                    thread_id: format!("thread-{index}"),
                    title: Some(format!("Thread {index}")),
                    workspace: test_workspace("workspace"),
                    workspace_id: None,
                    root_id: None,
                    provider: Some("mock".to_string()),
                    model: Some("mock".to_string()),
                    selection_mode: None,
                    tool_allowlist: Vec::new(),
                    developer_instructions: None,
                    external_tools: Vec::new(),
                    runner_destination: None,
                    runner_state: None,
                    runner_binding: None,
                    created_at: timestamp,
                    updated_at: timestamp,
                    message_count: index as u32,
                    usage: None,
                })
                .await
                .unwrap();
        }
        fs::remove_dir_all(base_path.join("thread-1"))
            .await
            .unwrap();

        let page = store
            .list_threads_page(ThreadListOptions {
                limit: Some(2),
                cursor: None,
            })
            .await
            .unwrap();

        assert_eq!(page.threads.len(), 1);
        assert_eq!(page.threads[0].thread_id, "thread-0");
        let index = store.load_thread_index().await.unwrap().unwrap();
        assert_eq!(index.threads.len(), 1);
        assert_eq!(index.threads[0].thread_id, "thread-0");

        let _ = fs::remove_dir_all(base_path).await;
    }

    #[tokio::test]
    async fn first_user_message_names_untitled_thread() {
        let base_path =
            std::env::temp_dir().join(format!("roder-jsonl-title-test-{}", uuid::Uuid::new_v4()));
        let store = JsonlThreadStore {
            base_path: base_path.clone(),
        };
        let thread_id = "thread-title".to_string();
        let turn_id = "turn-title".to_string();
        let now = OffsetDateTime::UNIX_EPOCH;

        store
            .create_thread(ThreadMetadata {
                thread_id: thread_id.clone(),
                title: None,
                workspace: test_workspace("workspace-gode"),
                workspace_id: None,
                root_id: None,
                provider: Some("mock".to_string()),
                model: Some("mock".to_string()),
                selection_mode: None,
                tool_allowlist: Vec::new(),
                developer_instructions: None,
                external_tools: Vec::new(),
                runner_destination: None,
                runner_state: None,
                runner_binding: None,
                created_at: now,
                updated_at: now,
                message_count: 0,
                usage: None,
            })
            .await
            .unwrap();
        append_test_turn_item(
            &store,
            &thread_id,
            &turn_id,
            1,
            TranscriptItem::UserMessage(UserMessage::text(
                "please make resume threads easier to find",
            )),
            now,
        )
        .await;

        let threads = store.list_threads().await.unwrap();
        let snapshot = store.load_thread(&thread_id).await.unwrap().unwrap();

        assert_eq!(
            threads[0].title.as_deref(),
            Some("please make resume threads easier to find")
        );
        assert_eq!(
            snapshot.metadata.unwrap().title.as_deref(),
            Some("please make resume threads easier to find")
        );

        let _ = fs::remove_dir_all(base_path).await;
    }

    #[tokio::test]
    async fn list_threads_rejects_malformed_metadata() {
        let base_path = std::env::temp_dir().join(format!(
            "roder-jsonl-recover-list-test-{}",
            uuid::Uuid::new_v4()
        ));
        let store = JsonlThreadStore {
            base_path: base_path.clone(),
        };
        let thread_id = "thread-recover-list".to_string();
        let turn_id = "turn-recover-list".to_string();
        let now = OffsetDateTime::UNIX_EPOCH;

        store
            .create_thread(ThreadMetadata {
                thread_id: thread_id.clone(),
                title: Some("Will be corrupted".to_string()),
                workspace: test_workspace("workspace"),
                workspace_id: None,
                root_id: None,
                provider: Some("mock".to_string()),
                model: Some("mock".to_string()),
                selection_mode: None,
                tool_allowlist: Vec::new(),
                developer_instructions: None,
                external_tools: Vec::new(),
                runner_destination: None,
                runner_state: None,
                runner_binding: None,
                created_at: now,
                updated_at: now,
                message_count: 0,
                usage: None,
            })
            .await
            .unwrap();
        append_test_turn_item(
            &store,
            &thread_id,
            &turn_id,
            1,
            TranscriptItem::UserMessage(UserMessage::text("recover this thread")),
            now,
        )
        .await;
        append_test_turn_item(
            &store,
            &thread_id,
            &turn_id,
            2,
            TranscriptItem::AssistantMessage(AssistantMessage {
                text: "continuing".to_string(),
                phase: None,
            }),
            now,
        )
        .await;
        fs::write(
            store.thread_dir(&thread_id).join("metadata.json"),
            "{broken",
        )
        .await
        .unwrap();

        let error = store.list_threads().await.unwrap_err();

        assert!(error.to_string().contains("thread metadata invalid"));

        let _ = fs::remove_dir_all(base_path).await;
    }

    #[tokio::test]
    async fn load_thread_rejects_malformed_metadata() {
        let base_path = std::env::temp_dir().join(format!(
            "roder-jsonl-recover-load-test-{}",
            uuid::Uuid::new_v4()
        ));
        let store = JsonlThreadStore {
            base_path: base_path.clone(),
        };
        let thread_id = "thread-recover-load".to_string();
        let turn_id = "turn-recover-load".to_string();
        let now = OffsetDateTime::UNIX_EPOCH;

        store
            .create_thread(ThreadMetadata {
                thread_id: thread_id.clone(),
                title: None,
                workspace: test_workspace("workspace"),
                workspace_id: None,
                root_id: None,
                provider: Some("mock".to_string()),
                model: Some("mock".to_string()),
                selection_mode: None,
                tool_allowlist: Vec::new(),
                developer_instructions: None,
                external_tools: Vec::new(),
                runner_destination: None,
                runner_state: None,
                runner_binding: None,
                created_at: now,
                updated_at: now,
                message_count: 0,
                usage: None,
            })
            .await
            .unwrap();
        append_test_turn_item(
            &store,
            &thread_id,
            &turn_id,
            1,
            TranscriptItem::UserMessage(UserMessage::text("resume despite metadata")),
            now,
        )
        .await;
        fs::write(
            store.thread_dir(&thread_id).join("metadata.json"),
            "not json",
        )
        .await
        .unwrap();

        let error = store.load_thread(&thread_id).await.unwrap_err();

        assert!(error.to_string().contains("thread metadata invalid"));

        let _ = fs::remove_dir_all(base_path).await;
    }

    #[tokio::test]
    async fn load_thread_accepts_valid_metadata_with_trailing_garbage_and_repairs_file() {
        let base_path = std::env::temp_dir().join(format!(
            "roder-jsonl-trailing-metadata-test-{}",
            uuid::Uuid::new_v4()
        ));
        let store = JsonlThreadStore {
            base_path: base_path.clone(),
        };
        let thread_id = "thread-trailing-metadata".to_string();
        let now = OffsetDateTime::UNIX_EPOCH;

        let metadata = ThreadMetadata {
            thread_id: thread_id.clone(),
            title: Some("Recover trailing metadata".to_string()),
            workspace: test_workspace("workspace"),
            workspace_id: None,
            root_id: None,
            provider: Some("codex".to_string()),
            model: Some("gpt-5.5".to_string()),
            selection_mode: None,
            tool_allowlist: Vec::new(),
            developer_instructions: None,
            external_tools: Vec::new(),
            runner_destination: None,
            runner_state: None,
            runner_binding: None,
            created_at: now,
            updated_at: now,
            message_count: 1,
            usage: None,
        };
        let dir = store.thread_dir(&thread_id);
        fs::create_dir_all(&dir).await.unwrap();
        let mut corrupted = serde_json::to_string_pretty(&metadata).unwrap();
        corrupted.push('}');
        fs::write(dir.join("metadata.json"), corrupted)
            .await
            .unwrap();

        let snapshot = store.load_thread(&thread_id).await.unwrap().unwrap();
        let loaded = snapshot.metadata.unwrap();

        assert_eq!(loaded.thread_id, thread_id);
        assert_eq!(loaded.title.as_deref(), Some("Recover trailing metadata"));
        assert_eq!(loaded.provider.as_deref(), Some("codex"));
        assert_eq!(loaded.model.as_deref(), Some("gpt-5.5"));
        assert_eq!(loaded.message_count, 1);

        let repaired = fs::read(dir.join("metadata.json")).await.unwrap();
        serde_json::from_slice::<ThreadMetadata>(&repaired).unwrap();

        let _ = fs::remove_dir_all(base_path).await;
    }

    #[tokio::test]
    async fn append_event_rejects_malformed_metadata_for_transcript_items() {
        let base_path = std::env::temp_dir().join(format!(
            "roder-jsonl-recover-append-test-{}",
            uuid::Uuid::new_v4()
        ));
        let store = JsonlThreadStore {
            base_path: base_path.clone(),
        };
        let thread_id = "thread-recover-append".to_string();
        let turn_id = "turn-recover-append".to_string();
        let now = OffsetDateTime::UNIX_EPOCH;

        store
            .create_thread(ThreadMetadata {
                thread_id: thread_id.clone(),
                title: None,
                workspace: test_workspace("workspace"),
                workspace_id: None,
                root_id: None,
                provider: None,
                model: None,
                selection_mode: None,
                tool_allowlist: Vec::new(),
                developer_instructions: None,
                external_tools: Vec::new(),
                runner_destination: None,
                runner_state: None,
                runner_binding: None,
                created_at: now,
                updated_at: now,
                message_count: 0,
                usage: None,
            })
            .await
            .unwrap();
        fs::write(
            store.thread_dir(&thread_id).join("metadata.json"),
            "{\"thread_id\":\"thread-recover-append\"} trailing",
        )
        .await
        .unwrap();

        let error = store
            .append_event(
                &thread_id,
                &transcript_item_event(
                    1,
                    &thread_id,
                    &turn_id,
                    TranscriptItem::UserMessage(UserMessage::text("repair me")),
                    now,
                ),
            )
            .await
            .unwrap_err();

        assert!(error.to_string().contains("thread metadata invalid"));

        let _ = fs::remove_dir_all(base_path).await;
    }

    #[tokio::test]
    async fn load_thread_accepts_concatenated_jsonl_records() {
        let base_path =
            std::env::temp_dir().join(format!("roder-jsonl-concat-test-{}", uuid::Uuid::new_v4()));
        let store = JsonlThreadStore {
            base_path: base_path.clone(),
        };
        let thread_id = "thread-concat".to_string();
        let turn_id = "turn-concat".to_string();
        let now = OffsetDateTime::UNIX_EPOCH;

        store
            .create_thread(ThreadMetadata {
                thread_id: thread_id.clone(),
                title: Some("Concatenated jsonl".to_string()),
                workspace: test_workspace("workspace"),
                workspace_id: None,
                root_id: None,
                provider: Some("mock".to_string()),
                model: Some("mock".to_string()),
                selection_mode: None,
                tool_allowlist: Vec::new(),
                developer_instructions: None,
                external_tools: Vec::new(),
                runner_destination: None,
                runner_state: None,
                runner_binding: None,
                created_at: now,
                updated_at: now,
                message_count: 0,
                usage: None,
            })
            .await
            .unwrap();

        let dir = store.thread_dir(&thread_id);
        let first = transcript_item_event(
            1,
            &thread_id,
            &turn_id,
            TranscriptItem::UserMessage(UserMessage::text("first")),
            now,
        );
        let second = transcript_item_event(
            2,
            &thread_id,
            &turn_id,
            TranscriptItem::AssistantMessage(AssistantMessage {
                text: "second".to_string(),
                phase: None,
            }),
            now,
        );
        let concatenated = format!(
            "{}{}\n",
            serde_json::to_string(&first).unwrap(),
            serde_json::to_string(&second).unwrap()
        );
        fs::write(dir.join("events.jsonl"), concatenated)
            .await
            .unwrap();

        let snapshot = store.load_thread(&thread_id).await.unwrap().unwrap();

        assert_eq!(snapshot.turns.len(), 1);
        assert_eq!(snapshot.turns[0].items.len(), 2);

        let _ = fs::remove_dir_all(base_path).await;
    }

    #[tokio::test]
    async fn malformed_event_reports_file_and_line() {
        let base_path = std::env::temp_dir().join(format!(
            "roder-jsonl-malformed-test-{}",
            uuid::Uuid::new_v4()
        ));
        let store = JsonlThreadStore {
            base_path: base_path.clone(),
        };
        let thread_id = "thread-malformed".to_string();
        let now = OffsetDateTime::UNIX_EPOCH;

        store
            .create_thread(ThreadMetadata {
                thread_id: thread_id.clone(),
                title: Some("Malformed jsonl".to_string()),
                workspace: test_workspace("workspace"),
                workspace_id: None,
                root_id: None,
                provider: Some("mock".to_string()),
                model: Some("mock".to_string()),
                selection_mode: None,
                tool_allowlist: Vec::new(),
                developer_instructions: None,
                external_tools: Vec::new(),
                runner_destination: None,
                runner_state: None,
                runner_binding: None,
                created_at: now,
                updated_at: now,
                message_count: 0,
                usage: None,
            })
            .await
            .unwrap();
        fs::write(
            store.thread_dir(&thread_id).join("events.jsonl"),
            "{\"event_id\":\"broken\"\n",
        )
        .await
        .unwrap();

        let err = store.load_thread(&thread_id).await.unwrap_err();
        let rendered = format!("{err:#}");

        assert!(rendered.contains("parse event record in"));
        assert!(rendered.contains("events.jsonl:1"));

        let _ = fs::remove_dir_all(base_path).await;
    }

    #[tokio::test]
    async fn load_missing_thread_returns_none() {
        let base_path = std::env::temp_dir().join(format!(
            "roder-jsonl-missing-thread-test-{}",
            uuid::Uuid::new_v4()
        ));
        let store = JsonlThreadStore {
            base_path: base_path.clone(),
        };

        assert!(
            store
                .load_thread(&"missing".to_string())
                .await
                .unwrap()
                .is_none()
        );

        let _ = fs::remove_dir_all(base_path).await;
    }
}
