use std::collections::HashSet;
use std::sync::Arc;

use anyhow::bail;
use roder_api::extension::MemoryStoreId;
use roder_api::memory::{
    MemoryCitation, MemoryId, MemoryQuery, MemoryRecord, MemoryScope, MemorySearchResult,
    MemoryStore, MemoryStoreFactory,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;

use crate::HonchoMemoryConfig;
use crate::client::{HonchoClient, HonchoMessage};

pub const STORE_ID: &str = "honcho-memory";

/// Derived session ids are prefixed so memory sessions are recognizable next
/// to whatever other sessions live in the Honcho workspace.
const SESSION_PREFIX: &str = "roder-memory-";
/// Bound on `roder_superseded_by` chain walks; each memory update adds one
/// hop, so this caps how many historical updates `get` will traverse.
const MAX_SUPERSEDE_HOPS: usize = 16;

pub struct HonchoMemoryStoreFactory {
    config: HonchoMemoryConfig,
}

impl HonchoMemoryStoreFactory {
    pub fn new(config: HonchoMemoryConfig) -> Self {
        Self { config }
    }
}

impl MemoryStoreFactory for HonchoMemoryStoreFactory {
    fn id(&self) -> MemoryStoreId {
        STORE_ID.to_string()
    }

    fn create(&self) -> Arc<dyn MemoryStore> {
        Arc::new(HonchoMemoryStore::new(self.config.clone()))
    }
}

pub struct HonchoMemoryStore {
    config: HonchoMemoryConfig,
    client: HonchoClient,
    base_ready: tokio::sync::OnceCell<()>,
    ready_sessions: tokio::sync::Mutex<HashSet<String>>,
}

impl HonchoMemoryStore {
    pub fn new(config: HonchoMemoryConfig) -> Self {
        let client = HonchoClient::new(&config);
        Self {
            config,
            client,
            base_ready: tokio::sync::OnceCell::new(),
            ready_sessions: tokio::sync::Mutex::new(HashSet::new()),
        }
    }

    pub fn session_id_for_scope(&self, scope: &MemoryScope) -> String {
        match &self.config.session_id {
            Some(pinned) => pinned.clone(),
            None => format!("{SESSION_PREFIX}{}", sanitize_id(&scope.stable_id())),
        }
    }

    async fn ensure_base(&self) -> anyhow::Result<()> {
        self.base_ready
            .get_or_try_init(|| async {
                self.client
                    .ensure_workspace(&self.config.workspace_id)
                    .await?;
                self.client
                    .ensure_peer(&self.config.workspace_id, &self.config.peer_id)
                    .await?;
                Ok::<(), anyhow::Error>(())
            })
            .await?;
        Ok(())
    }

    async fn ensure_session(&self, session: &str) -> anyhow::Result<()> {
        self.ensure_base().await?;
        let mut ready = self.ready_sessions.lock().await;
        if ready.contains(session) {
            return Ok(());
        }
        self.client
            .ensure_session(
                &self.config.workspace_id,
                session,
                &self.config.peer_id,
                json!({ "roder_memory": true }),
            )
            .await?;
        ready.insert(session.to_string());
        Ok(())
    }

    async fn create(&self, record: MemoryRecord) -> anyhow::Result<MemoryId> {
        let session = self.session_id_for_scope(&record.scope);
        self.ensure_session(&session).await?;
        let metadata = message_metadata(&record);
        let message = self
            .client
            .add_message(
                &self.config.workspace_id,
                &session,
                &self.config.peer_id,
                &record.text,
                metadata,
                &format_time(record.created_at),
            )
            .await?;
        Ok(compose_id(&session, &message.id))
    }

    /// Honcho message content is immutable, so an update writes a fresh
    /// message and tombstones the old one with a `roder_superseded_by`
    /// pointer; `get`/`delete` follow the pointer chain to the live head.
    async fn update_existing(
        &self,
        id: MemoryId,
        record: MemoryRecord,
    ) -> anyhow::Result<MemoryId> {
        let Some((session, message)) = self.resolve_head(&id).await? else {
            bail!("memory not found: {id}");
        };
        let new_id = self.create(MemoryRecord { id: None, ..record }).await?;
        let mut metadata = message.metadata.clone();
        merge_metadata(
            &mut metadata,
            json!({
                "roder_deleted": true,
                "roder_superseded_by": new_id,
                "roder_updated_at": format_time(OffsetDateTime::now_utc()),
            }),
        );
        self.client
            .update_message_metadata(&self.config.workspace_id, &session, &message.id, metadata)
            .await?;
        Ok(new_id)
    }

    /// Follows the supersede chain starting at `id` and returns the final
    /// message (live head, or a plain tombstone if the memory was deleted).
    async fn resolve_chain(&self, id: &str) -> anyhow::Result<Option<(String, HonchoMessage)>> {
        let Some((mut session, mut message_id)) = split_id(id) else {
            return Ok(None);
        };
        for _ in 0..MAX_SUPERSEDE_HOPS {
            let Some(message) = self
                .client
                .get_message(&self.config.workspace_id, &session, &message_id)
                .await?
            else {
                return Ok(None);
            };
            // Direct id lookups bypass the filtered list/search paths, so
            // authorship is enforced here too — an id pointing at another
            // peer's message resolves to "not found".
            if message.peer_id != self.config.peer_id {
                return Ok(None);
            }
            let superseded_by = message
                .metadata
                .get("roder_superseded_by")
                .and_then(Value::as_str)
                .map(str::to_string);
            let deleted = metadata_flag(&message.metadata, "roder_deleted");
            match superseded_by {
                Some(next) if deleted => {
                    let Some((next_session, next_message)) = split_id(&next) else {
                        return Ok(Some((session, message)));
                    };
                    session = next_session;
                    message_id = next_message;
                }
                _ => return Ok(Some((session, message))),
            }
        }
        bail!("memory supersede chain exceeded {MAX_SUPERSEDE_HOPS} hops for {id}");
    }

    /// Like `resolve_chain` but only returns live (non-deleted) heads.
    async fn resolve_head(&self, id: &str) -> anyhow::Result<Option<(String, HonchoMessage)>> {
        let Some((session, message)) = self.resolve_chain(id).await? else {
            return Ok(None);
        };
        if metadata_flag(&message.metadata, "roder_deleted") {
            return Ok(None);
        }
        Ok(Some((session, message)))
    }

    async fn list_records(
        &self,
        scope: Option<&MemoryScope>,
        limit: usize,
    ) -> anyhow::Result<Vec<MemoryRecord>> {
        self.ensure_base().await?;
        let size = limit.clamp(1, 100);
        let mut records = Vec::new();
        match scope {
            Some(scope) => {
                let session = self.session_id_for_scope(scope);
                let filters = live_filters(&self.config.peer_id, Some(&scope.stable_id()));
                let messages = self
                    .client
                    .list_session_messages(&self.config.workspace_id, &session, filters, size)
                    .await?;
                records.extend(records_from_messages(
                    messages,
                    &self.config.peer_id,
                    Some(scope),
                ));
            }
            None => {
                let sessions = match &self.config.session_id {
                    Some(pinned) => vec![pinned.clone()],
                    None => {
                        self.client
                            .list_sessions(
                                &self.config.workspace_id,
                                json!({ "metadata": { "roder_memory": true } }),
                                100,
                            )
                            .await?
                    }
                };
                for session in sessions {
                    if records.len() >= size {
                        break;
                    }
                    let messages = self
                        .client
                        .list_session_messages(
                            &self.config.workspace_id,
                            &session,
                            live_filters(&self.config.peer_id, None),
                            size,
                        )
                        .await?;
                    records.extend(records_from_messages(messages, &self.config.peer_id, None));
                }
            }
        }
        records.sort_by_key(|record| std::cmp::Reverse(record.updated_at));
        records.truncate(size);
        Ok(records)
    }
}

#[async_trait::async_trait]
impl MemoryStore for HonchoMemoryStore {
    fn id(&self) -> MemoryStoreId {
        STORE_ID.to_string()
    }

    async fn put(&self, record: MemoryRecord) -> anyhow::Result<MemoryId> {
        match record.id.clone() {
            Some(id) => self.update_existing(id, record).await,
            None => self.create(record).await,
        }
    }

    async fn get(&self, id: &MemoryId) -> anyhow::Result<Option<MemoryRecord>> {
        let Some((session, message)) = self.resolve_chain(id).await? else {
            return Ok(None);
        };
        Ok(record_from_message(&message, &session))
    }

    /// Retrieval is delegated to Honcho's hosted semantic search; the store
    /// never embeds locally, so `query.provider_id`/`query.model` are
    /// ignored. Honcho does not return similarity scores, so scores are
    /// rank-derived.
    async fn search(&self, query: MemoryQuery) -> anyhow::Result<Vec<MemorySearchResult>> {
        let limit = query.limit.max(1);
        if query.text.trim().is_empty() {
            let mut records = self.list_records(query.scope.as_ref(), limit).await?;
            if query.include_global
                && query.scope.as_ref() != Some(&MemoryScope::Global)
                && query.scope.is_some()
            {
                records.extend(self.list_records(Some(&MemoryScope::Global), limit).await?);
                // Each batch arrives sorted internally; without a merged
                // re-rank the global batch is starved exactly when the scope
                // batch fills the limit.
                records.sort_by_key(|record| std::cmp::Reverse(record.updated_at));
            }
            records.truncate(limit);
            return Ok(records
                .into_iter()
                .map(|record| search_result(record, 1.0))
                .collect());
        }

        self.ensure_base().await?;
        let mut scope_ids: Vec<Option<String>> = match &query.scope {
            Some(scope) => {
                let mut ids = vec![Some(scope.stable_id())];
                if query.include_global && *scope != MemoryScope::Global {
                    ids.push(Some(MemoryScope::Global.stable_id()));
                }
                ids
            }
            None => vec![None],
        };
        let mut results = Vec::new();
        for scope_id in scope_ids.drain(..) {
            let filters = live_filters(&self.config.peer_id, scope_id.as_deref());
            let messages = self
                .client
                .search_workspace(&self.config.workspace_id, &query.text, filters, limit)
                .await?;
            for (rank, message) in messages.iter().enumerate() {
                // Authorship is re-checked client-side (symmetric with
                // `get`/`delete`): a search backend that ignores the
                // `peer_id` filter must not surface another peer's records.
                if message.peer_id != self.config.peer_id {
                    continue;
                }
                let Some(record) = record_from_message(message, &message.session_id) else {
                    continue;
                };
                if record.deleted {
                    continue;
                }
                if let Some(scope_id) = scope_id.as_deref()
                    && record.scope.stable_id() != scope_id
                {
                    continue;
                }
                results.push(search_result(record, 1.0 / (1.0 + rank as f32)));
            }
        }
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(limit);
        Ok(results)
    }

    /// Soft delete, mirroring the sqlite store: the message is tombstoned via
    /// metadata, never removed from Honcho.
    async fn delete(&self, id: &MemoryId) -> anyhow::Result<()> {
        let Some((session, message)) = self.resolve_head(id).await? else {
            return Ok(());
        };
        let mut metadata = message.metadata.clone();
        merge_metadata(
            &mut metadata,
            json!({
                "roder_deleted": true,
                "roder_updated_at": format_time(OffsetDateTime::now_utc()),
            }),
        );
        self.client
            .update_message_metadata(&self.config.workspace_id, &session, &message.id, metadata)
            .await?;
        Ok(())
    }

    async fn list(
        &self,
        scope: Option<MemoryScope>,
        limit: usize,
    ) -> anyhow::Result<Vec<MemoryRecord>> {
        self.list_records(scope.as_ref(), limit).await
    }
}

/// Equality-only filters; Honcho's filter language matches message columns
/// and nested metadata keys. `peer_id` pins reads to memories this store
/// authored — scope ids (and the default tool scope) are not unique per
/// runtime, so without it stores sharing a workspace under distinct peers
/// would read each other's records. `roder_deleted` is written on every
/// record so tombstones can be excluded with an equality match.
fn live_filters(peer_id: &str, scope_id: Option<&str>) -> Value {
    let mut metadata = json!({ "roder_memory": true, "roder_deleted": false });
    if let Some(scope_id) = scope_id {
        metadata["roder_scope"] = Value::String(scope_id.to_string());
    }
    json!({ "peer_id": peer_id, "metadata": metadata })
}

fn message_metadata(record: &MemoryRecord) -> Value {
    let hash = record
        .content_hash
        .clone()
        .unwrap_or_else(|| content_hash(&record.text));
    json!({
        "roder_memory": true,
        "roder_scope": record.scope.stable_id(),
        "roder_content_hash": hash,
        "roder_metadata": record.metadata,
        "roder_deleted": false,
        "roder_created_at": format_time(record.created_at),
        "roder_updated_at": format_time(OffsetDateTime::now_utc()),
    })
}

fn merge_metadata(metadata: &mut Value, patch: Value) {
    let (Some(object), Some(patch)) = (metadata.as_object_mut(), patch.as_object()) else {
        return;
    };
    for (key, value) in patch {
        object.insert(key.clone(), value.clone());
    }
}

fn metadata_flag(metadata: &Value, key: &str) -> bool {
    metadata.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// `peer_id` re-applies the server-side authorship filter client-side so the
/// list paths stay symmetric with `get`/`delete` even against a backend that
/// ignores the `peer_id` filter key.
fn records_from_messages(
    messages: Vec<HonchoMessage>,
    peer_id: &str,
    scope: Option<&MemoryScope>,
) -> Vec<MemoryRecord> {
    messages
        .iter()
        .filter(|message| message.peer_id == peer_id)
        .filter_map(|message| record_from_message(message, &message.session_id))
        .filter(|record| !record.deleted)
        .filter(|record| scope.is_none_or(|scope| record.scope == *scope))
        .collect()
}

fn record_from_message(message: &HonchoMessage, session: &str) -> Option<MemoryRecord> {
    let metadata = &message.metadata;
    if !metadata_flag(metadata, "roder_memory") {
        return None;
    }
    let scope = metadata
        .get("roder_scope")
        .and_then(Value::as_str)
        .and_then(parse_stable_id)?;
    let created_at = metadata
        .get("roder_created_at")
        .and_then(Value::as_str)
        .or(message.created_at.as_deref())
        .and_then(|value| parse_time(value).ok())
        .unwrap_or(OffsetDateTime::UNIX_EPOCH);
    let updated_at = metadata
        .get("roder_updated_at")
        .and_then(Value::as_str)
        .and_then(|value| parse_time(value).ok())
        .unwrap_or(created_at);
    Some(MemoryRecord {
        id: Some(compose_id(session, &message.id)),
        scope,
        text: message.content.clone(),
        content_hash: metadata
            .get("roder_content_hash")
            .and_then(Value::as_str)
            .map(str::to_string),
        metadata: metadata
            .get("roder_metadata")
            .cloned()
            .unwrap_or(Value::Null),
        usage: None,
        deleted: metadata_flag(metadata, "roder_deleted"),
        created_at,
        updated_at,
    })
}

fn search_result(record: MemoryRecord, score: f32) -> MemorySearchResult {
    let citation = record.id.clone().map(|memory_id| MemoryCitation {
        memory_id,
        scope_id: record.scope.stable_id(),
        snippet: snippet(&record.text),
        score_millis: (score.max(0.0) * 1000.0) as u32,
    });
    MemorySearchResult {
        record,
        score,
        citation,
    }
}

/// Memory ids are `{honcho_session_id}/{honcho_message_id}` so a record can
/// be addressed without a session lookup. Honcho ids never contain `/`.
pub fn compose_id(session: &str, message: &str) -> MemoryId {
    format!("{session}/{message}")
}

fn split_id(id: &str) -> Option<(String, String)> {
    let (session, message) = id.split_once('/')?;
    if session.is_empty() || message.is_empty() {
        return None;
    }
    Some((session.to_string(), message.to_string()))
}

fn parse_stable_id(stable_id: &str) -> Option<MemoryScope> {
    if stable_id == "global" {
        return Some(MemoryScope::Global);
    }
    let (kind, value) = stable_id.split_once(':')?;
    let value = value.to_string();
    match kind {
        "user" => Some(MemoryScope::User(value)),
        "workspace" => Some(MemoryScope::Workspace(value)),
        "project" => Some(MemoryScope::Project(value)),
        "thread" => Some(MemoryScope::Thread(value)),
        _ => None,
    }
}

/// Honcho resource ids only allow `[A-Za-z0-9_-]`; collisions after
/// sanitization are harmless because the exact scope travels in message
/// metadata and every read path filters on it.
fn sanitize_id(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

fn snippet(text: &str) -> String {
    const MAX: usize = 180;
    if text.chars().count() <= MAX {
        text.to_string()
    } else {
        let mut out = text.chars().take(MAX).collect::<String>();
        out.push_str("...");
        out
    }
}

fn content_hash(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn format_time(time: OffsetDateTime) -> String {
    time.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| OffsetDateTime::UNIX_EPOCH.to_string())
}

fn parse_time(input: &str) -> anyhow::Result<OffsetDateTime> {
    Ok(OffsetDateTime::parse(
        input,
        &time::format_description::well_known::Rfc3339,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(session_id: Option<&str>) -> HonchoMemoryConfig {
        HonchoMemoryConfig {
            api_key: "key".to_string(),
            base_url: "http://localhost:1".to_string(),
            workspace_id: "ws".to_string(),
            peer_id: "peer".to_string(),
            session_id: session_id.map(str::to_string),
        }
    }

    #[test]
    fn scope_maps_to_sanitized_session_id() {
        let store = HonchoMemoryStore::new(config(None));
        assert_eq!(
            store.session_id_for_scope(&MemoryScope::Global),
            "roder-memory-global"
        );
        assert_eq!(
            store.session_id_for_scope(&MemoryScope::Workspace("a b/c".to_string())),
            "roder-memory-workspace-a-b-c"
        );
    }

    #[test]
    fn pinned_session_overrides_scope_mapping() {
        let store = HonchoMemoryStore::new(config(Some("pinned")));
        assert_eq!(
            store.session_id_for_scope(&MemoryScope::Thread("t".to_string())),
            "pinned"
        );
    }

    #[test]
    fn memory_ids_round_trip_session_and_message() {
        let id = compose_id("roder-memory-global", "msg-1");
        assert_eq!(
            split_id(&id),
            Some(("roder-memory-global".to_string(), "msg-1".to_string()))
        );
        assert_eq!(split_id("missing-separator"), None);
    }

    #[test]
    fn stable_ids_round_trip_scopes() {
        for scope in [
            MemoryScope::Global,
            MemoryScope::User("u".to_string()),
            MemoryScope::Workspace("w".to_string()),
            MemoryScope::Project("p".to_string()),
            MemoryScope::Thread("t".to_string()),
        ] {
            assert_eq!(parse_stable_id(&scope.stable_id()), Some(scope));
        }
        assert_eq!(parse_stable_id("nonsense"), None);
    }

    #[test]
    fn record_round_trips_through_message_metadata() {
        let record = MemoryRecord {
            id: None,
            scope: MemoryScope::Project("p".to_string()),
            text: "remember this".to_string(),
            content_hash: None,
            metadata: serde_json::json!({ "source": "test" }),
            usage: None,
            deleted: false,
            created_at: OffsetDateTime::now_utc(),
            updated_at: OffsetDateTime::now_utc(),
        };
        let message = HonchoMessage {
            id: "msg-1".to_string(),
            content: record.text.clone(),
            peer_id: "peer".to_string(),
            session_id: "session".to_string(),
            metadata: message_metadata(&record),
            created_at: None,
        };
        let parsed = record_from_message(&message, "session").unwrap();
        assert_eq!(parsed.scope, record.scope);
        assert_eq!(parsed.text, record.text);
        assert_eq!(parsed.metadata, record.metadata);
        assert_eq!(parsed.id.as_deref(), Some("session/msg-1"));
        assert!(!parsed.deleted);
        assert_eq!(parsed.content_hash, Some(content_hash(&record.text)));
    }
}
