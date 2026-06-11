use std::sync::Arc;

use anyhow::Context;
use roder_api::artifacts::ContextArtifactStore;
use roder_api::events::{EventEnvelope, RoderEvent, ThreadId};
use roder_api::extension::ThreadStoreId;
use roder_api::extension_state::ExtensionStateRecord;
use roder_api::thread::{
    ThreadItemEvent, ThreadListOptions, ThreadListPage, ThreadMetadata, ThreadSnapshot,
    ThreadStore, ThreadStoreFactory, project_turns_from_events, validate_thread_workspace,
};
use roder_api::transcript::TranscriptItem;
use sqlx_core::pool::Pool;
use sqlx_core::row::Row;
use sqlx_postgres::Postgres;
use time::OffsetDateTime;

use crate::artifacts::PostgresArtifactStore;
use crate::schema;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostgresSessionConfig {
    pub database_url: String,
    pub tenant_id: String,
    pub max_connections: Option<u32>,
}

impl PostgresSessionConfig {
    pub fn new(
        database_url: impl Into<String>,
        tenant_id: impl Into<String>,
    ) -> anyhow::Result<Self> {
        let config = Self {
            database_url: database_url.into(),
            tenant_id: tenant_id.into(),
            max_connections: None,
        };
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.database_url.trim().is_empty(),
            "PostgreSQL session database URL is required"
        );
        anyhow::ensure!(
            !self.tenant_id.trim().is_empty(),
            "PostgreSQL session tenant id is required"
        );
        anyhow::ensure!(
            !self.tenant_id.contains('/'),
            "PostgreSQL session tenant id cannot contain '/'"
        );
        Ok(())
    }

    pub fn redacted_database_url(&self) -> String {
        redact_database_url(&self.database_url)
    }
}

/// Validates a tenant id for store scoping (shared by config validation
/// and `for_tenant` handles).
pub fn validate_tenant_id(tenant_id: &str) -> anyhow::Result<String> {
    let tenant_id = tenant_id.trim();
    anyhow::ensure!(!tenant_id.is_empty(), "tenant id is required");
    anyhow::ensure!(
        !tenant_id.contains('/'),
        "tenant id cannot contain '/': {tenant_id}"
    );
    Ok(tenant_id.to_string())
}

pub fn redact_database_url(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return "<redacted>".to_string();
    };
    let Some((auth_host, tail)) = rest.split_once('@') else {
        return format!("{scheme}://{rest}");
    };
    let user = auth_host
        .split_once(':')
        .map(|(user, _)| user)
        .unwrap_or(auth_host);
    format!("{scheme}://{user}:<redacted>@{tail}")
}

#[derive(Clone)]
pub struct PostgresSessionStore {
    pool: Pool<Postgres>,
    tenant_id: String,
}

impl PostgresSessionStore {
    pub async fn connect(config: &PostgresSessionConfig) -> anyhow::Result<Self> {
        config.validate()?;
        let pool = sqlx_postgres::PgPoolOptions::new()
            .max_connections(config.max_connections.unwrap_or(5))
            .connect(&config.database_url)
            .await
            .with_context(|| {
                format!(
                    "connect to PostgreSQL session store at {}",
                    config.redacted_database_url()
                )
            })?;
        schema::migrate(&pool).await.with_context(|| {
            format!(
                "migrate PostgreSQL session store at {}",
                config.redacted_database_url()
            )
        })?;
        Ok(Self {
            pool,
            tenant_id: config.tenant_id.clone(),
        })
    }

    /**
     * Derives a tenant-scoped handle that shares this store's connection
     * pool (roadmap phase 72, Task 3). Hosted deployments connect once and
     * mint per-tenant handles; the tenant id is fixed at construction from
     * the authenticated request context — request payloads can never pick
     * a tenant because every query is bound to `self.tenant_id`.
     */
    pub fn for_tenant(&self, tenant_id: &str) -> anyhow::Result<Self> {
        let tenant_id = validate_tenant_id(tenant_id)?;
        Ok(Self {
            pool: self.pool.clone(),
            tenant_id,
        })
    }

    /// The tenant this handle is bound to.
    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }

    fn artifact_store(&self) -> ContextArtifactStore {
        ContextArtifactStore::new(Arc::new(PostgresArtifactStore {
            pool: self.pool.clone(),
            tenant_id: self.tenant_id.clone(),
        }))
    }

    async fn metadata_for_thread_item(
        &self,
        thread_id: &ThreadId,
        item: &TranscriptItem,
    ) -> anyhow::Result<()> {
        if !matches!(
            item,
            TranscriptItem::UserMessage(_) | TranscriptItem::AssistantMessage(_)
        ) {
            return Ok(());
        }
        let mut metadata = match self.load_metadata(thread_id).await? {
            Some(metadata) => metadata,
            None => anyhow::bail!("thread metadata missing for {thread_id}"),
        };
        metadata.updated_at = OffsetDateTime::now_utc();
        metadata.message_count = metadata.message_count.saturating_add(1);
        if metadata
            .title
            .as_ref()
            .is_none_or(|title| title.trim().is_empty())
            && let TranscriptItem::UserMessage(message) = item
        {
            metadata.title = title_from_user_text(&message.text);
        }
        self.update_thread_metadata(metadata).await?;
        Ok(())
    }

    async fn load_metadata(&self, thread_id: &ThreadId) -> anyhow::Result<Option<ThreadMetadata>> {
        let row = sqlx_core::query::query::<Postgres>("SELECT metadata FROM roder_sessions WHERE tenant_id = $1 AND thread_id = $2 AND archived = FALSE")
            .bind(&self.tenant_id).bind(thread_id).fetch_optional(&self.pool).await?;
        row.map(|row| {
            let json: sqlx_core::types::Json<ThreadMetadata> = row.try_get("metadata")?;
            Ok(json.0)
        })
        .transpose()
    }
}

#[async_trait::async_trait]
impl ThreadStore for PostgresSessionStore {
    fn id(&self) -> ThreadStoreId {
        "postgres-session".to_string()
    }

    fn context_artifact_store(&self) -> Option<ContextArtifactStore> {
        Some(self.artifact_store())
    }

    async fn create_thread(&self, metadata: ThreadMetadata) -> anyhow::Result<ThreadMetadata> {
        validate_thread_workspace(&metadata.workspace)?;
        sqlx_core::query::query::<Postgres>("INSERT INTO roder_sessions (tenant_id, thread_id, metadata, archived, created_at, updated_at) VALUES ($1,$2,$3,FALSE,$4,$5) ON CONFLICT (tenant_id, thread_id) DO UPDATE SET metadata = EXCLUDED.metadata, archived = FALSE, updated_at = EXCLUDED.updated_at")
            .bind(&self.tenant_id).bind(&metadata.thread_id).bind(sqlx_core::types::Json(&metadata)).bind(metadata.created_at).bind(metadata.updated_at).execute(&self.pool).await?;
        Ok(metadata)
    }

    async fn update_thread_metadata(
        &self,
        metadata: ThreadMetadata,
    ) -> anyhow::Result<ThreadMetadata> {
        validate_thread_workspace(&metadata.workspace)?;
        sqlx_core::query::query::<Postgres>("UPDATE roder_sessions SET metadata = $1, updated_at = $2 WHERE tenant_id = $3 AND thread_id = $4 AND archived = FALSE")
            .bind(sqlx_core::types::Json(&metadata)).bind(metadata.updated_at).bind(&self.tenant_id).bind(&metadata.thread_id).execute(&self.pool).await?;
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
        let offset = options
            .cursor
            .as_deref()
            .and_then(|cursor| cursor.parse::<i64>().ok())
            .unwrap_or(0)
            .max(0);
        let limit = options.limit.map(|limit| limit as i64).unwrap_or(i64::MAX);
        let rows = sqlx_core::query::query::<Postgres>("SELECT metadata FROM roder_sessions WHERE tenant_id = $1 AND archived = FALSE ORDER BY updated_at DESC LIMIT $2 OFFSET $3")
            .bind(&self.tenant_id)
            .bind(limit.saturating_add(1))
            .bind(offset)
            .fetch_all(&self.pool)
            .await?;
        let mut threads = rows
            .into_iter()
            .map(|row| {
                let json: sqlx_core::types::Json<ThreadMetadata> = row.try_get("metadata")?;
                Ok(json.0)
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let has_more = options.limit.is_some_and(|limit| threads.len() > limit);
        if let Some(limit) = options.limit {
            threads.truncate(limit);
        }
        Ok(ThreadListPage {
            threads,
            next_cursor: has_more.then(|| (offset + limit).to_string()),
            backwards_cursor: (offset > 0).then(|| offset.saturating_sub(limit).to_string()),
        })
    }

    async fn load_thread(&self, thread_id: &ThreadId) -> anyhow::Result<Option<ThreadSnapshot>> {
        let Some(metadata) = self.load_metadata(thread_id).await? else {
            return Ok(None);
        };
        let event_rows = sqlx_core::query::query::<Postgres>("SELECT event FROM roder_session_events WHERE tenant_id = $1 AND thread_id = $2 ORDER BY seq ASC")
            .bind(&self.tenant_id).bind(thread_id).fetch_all(&self.pool).await?;
        let events = event_rows
            .into_iter()
            .map(|row| {
                let json: sqlx_core::types::Json<EventEnvelope> = row.try_get("event")?;
                Ok(json.0)
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let turns = project_turns_from_events(thread_id, &events);
        let item_rows = sqlx_core::query::query::<Postgres>("SELECT item_event FROM roder_session_item_events WHERE tenant_id = $1 AND thread_id = $2 ORDER BY seq ASC")
            .bind(&self.tenant_id).bind(thread_id).fetch_all(&self.pool).await?;
        let item_events = item_rows
            .into_iter()
            .map(|row| {
                let json: sqlx_core::types::Json<ThreadItemEvent> = row.try_get("item_event")?;
                Ok(json.0)
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let state_rows = sqlx_core::query::query::<Postgres>("SELECT record FROM roder_session_extension_state WHERE tenant_id = $1 AND thread_id = $2 ORDER BY seq ASC")
            .bind(&self.tenant_id).bind(thread_id).fetch_all(&self.pool).await?;
        let extension_states = state_rows
            .into_iter()
            .map(|row| {
                let json: sqlx_core::types::Json<ExtensionStateRecord> = row.try_get("record")?;
                Ok(json.0)
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(Some(ThreadSnapshot {
            metadata: Some(metadata),
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
        self.load_metadata(thread_id).await
    }

    async fn archive_thread(&self, thread_id: &ThreadId) -> anyhow::Result<bool> {
        let result = sqlx_core::query::query::<Postgres>("UPDATE roder_sessions SET archived = TRUE, updated_at = now() WHERE tenant_id = $1 AND thread_id = $2 AND archived = FALSE")
            .bind(&self.tenant_id).bind(thread_id).execute(&self.pool).await?;
        Ok(result.rows_affected() > 0)
    }

    async fn append_event(
        &self,
        thread_id: &ThreadId,
        envelope: &EventEnvelope,
    ) -> anyhow::Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx_core::query::query::<Postgres>("INSERT INTO roder_session_events (tenant_id, thread_id, seq, event) VALUES ($1,$2,$3,$4) ON CONFLICT (tenant_id, thread_id, seq) DO UPDATE SET event = EXCLUDED.event")
            .bind(&self.tenant_id).bind(thread_id).bind(envelope.seq as i64).bind(sqlx_core::types::Json(envelope)).execute(&mut *tx).await?;
        tx.commit().await?;
        if let RoderEvent::TranscriptItemAppended(event) = &envelope.event
            && let Some(item) = &event.item
        {
            self.metadata_for_thread_item(thread_id, item).await?;
        }
        Ok(())
    }

    async fn append_item_event(
        &self,
        thread_id: &ThreadId,
        item_event: &ThreadItemEvent,
    ) -> anyhow::Result<()> {
        sqlx_core::query::query::<Postgres>("INSERT INTO roder_session_item_events (tenant_id, thread_id, seq, item_event) VALUES ($1,$2,$3,$4) ON CONFLICT (tenant_id, thread_id, seq) DO UPDATE SET item_event = EXCLUDED.item_event")
            .bind(&self.tenant_id).bind(thread_id).bind(item_event.seq as i64).bind(sqlx_core::types::Json(item_event)).execute(&self.pool).await?;
        Ok(())
    }

    async fn append_extension_state(
        &self,
        thread_id: &ThreadId,
        record: &ExtensionStateRecord,
    ) -> anyhow::Result<()> {
        sqlx_core::query::query::<Postgres>("INSERT INTO roder_session_extension_state (tenant_id, thread_id, record) VALUES ($1,$2,$3)")
            .bind(&self.tenant_id).bind(thread_id).bind(sqlx_core::types::Json(record)).execute(&self.pool).await?;
        Ok(())
    }
}

pub struct PostgresSessionStoreFactory {
    pub config: PostgresSessionConfig,
}

impl ThreadStoreFactory for PostgresSessionStoreFactory {
    fn id(&self) -> ThreadStoreId {
        "postgres-session".to_string()
    }
    fn create(&self) -> Arc<dyn ThreadStore> {
        let config = self.config.clone();
        let store = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(async move { PostgresSessionStore::connect(&config).await })
        })
        .unwrap_or_else(|err| panic!("failed to initialize PostgreSQL session store: {}", err));
        Arc::new(store)
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
