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
use sqlx_mysql::MySql;
use time::OffsetDateTime;

use crate::artifacts::MysqlArtifactStore;
use crate::executor::DbExecutor;
use crate::schema;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MysqlSessionConfig {
    pub database_url: String,
    pub tenant_id: String,
    pub max_connections: Option<u32>,
}

impl MysqlSessionConfig {
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
            "MySQL session database URL is required"
        );
        anyhow::ensure!(
            !self.tenant_id.trim().is_empty(),
            "MySQL session tenant id is required"
        );
        anyhow::ensure!(
            !self.tenant_id.contains('/'),
            "MySQL session tenant id cannot contain '/'"
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

pub(crate) fn unix_micros(timestamp: OffsetDateTime) -> i64 {
    (timestamp.unix_timestamp_nanos() / 1_000) as i64
}

pub(crate) fn unix_micros_now() -> i64 {
    unix_micros(OffsetDateTime::now_utc())
}

#[derive(Clone)]
pub struct MysqlSessionStore {
    executor: Arc<DbExecutor>,
    pool: Pool<MySql>,
    tenant_id: String,
}

impl MysqlSessionStore {
    pub async fn connect(config: &MysqlSessionConfig) -> anyhow::Result<Self> {
        let executor = DbExecutor::new()?;
        Self::connect_on(executor, config.clone()).await
    }

    /// Synchronous connect for sync factory contexts; the work still runs on
    /// the store's dedicated runtime.
    pub fn connect_blocking(config: &MysqlSessionConfig) -> anyhow::Result<Self> {
        let executor = DbExecutor::new()?;
        let config = config.clone();
        let pool = executor.run_blocking(open_pool(config.clone()))?;
        Ok(Self {
            executor,
            pool,
            tenant_id: config.tenant_id,
        })
    }

    async fn connect_on(
        executor: Arc<DbExecutor>,
        config: MysqlSessionConfig,
    ) -> anyhow::Result<Self> {
        let pool = executor.run(open_pool(config.clone())).await?;
        Ok(Self {
            executor,
            pool,
            tenant_id: config.tenant_id,
        })
    }

    /// Derives a tenant-scoped handle sharing this store's pool and runtime,
    /// mirroring the PostgreSQL store's hosted-tenancy contract.
    pub fn for_tenant(&self, tenant_id: &str) -> anyhow::Result<Self> {
        let tenant_id = validate_tenant_id(tenant_id)?;
        Ok(Self {
            executor: self.executor.clone(),
            pool: self.pool.clone(),
            tenant_id,
        })
    }

    /// The tenant this handle is bound to.
    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }

    fn artifact_store(&self) -> ContextArtifactStore {
        ContextArtifactStore::new(Arc::new(MysqlArtifactStore {
            executor: self.executor.clone(),
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
        let pool = self.pool.clone();
        let tenant_id = self.tenant_id.clone();
        let thread_id = thread_id.clone();
        self.executor
            .run(async move { load_metadata_on(&pool, &tenant_id, &thread_id).await })
            .await
    }
}

async fn open_pool(config: MysqlSessionConfig) -> anyhow::Result<Pool<MySql>> {
    config.validate()?;
    let pool = sqlx_mysql::MySqlPoolOptions::new()
        .max_connections(config.max_connections.unwrap_or(5))
        .connect(&config.database_url)
        .await
        .with_context(|| {
            format!(
                "connect to MySQL session store at {}",
                config.redacted_database_url()
            )
        })?;
    schema::migrate(&pool).await.with_context(|| {
        format!(
            "migrate MySQL session store at {}",
            config.redacted_database_url()
        )
    })?;
    Ok(pool)
}

async fn load_metadata_on(
    pool: &Pool<MySql>,
    tenant_id: &str,
    thread_id: &str,
) -> anyhow::Result<Option<ThreadMetadata>> {
    let row = sqlx_core::query::query::<MySql>("SELECT metadata FROM roder_sessions WHERE tenant_id = ? AND thread_id = ? AND archived = FALSE")
        .bind(tenant_id).bind(thread_id).fetch_optional(pool).await?;
    row.map(|row| {
        let json: sqlx_core::types::Json<ThreadMetadata> = row.try_get("metadata")?;
        Ok(json.0)
    })
    .transpose()
}

#[async_trait::async_trait]
impl ThreadStore for MysqlSessionStore {
    fn id(&self) -> ThreadStoreId {
        "mysql-session".to_string()
    }

    fn context_artifact_store(&self) -> Option<ContextArtifactStore> {
        Some(self.artifact_store())
    }

    async fn create_thread(&self, metadata: ThreadMetadata) -> anyhow::Result<ThreadMetadata> {
        validate_thread_workspace(&metadata.workspace)?;
        let pool = self.pool.clone();
        let tenant_id = self.tenant_id.clone();
        let row_metadata = metadata.clone();
        self.executor
            .run(async move {
                sqlx_core::query::query::<MySql>(
                    "INSERT INTO roder_sessions (tenant_id, thread_id, metadata, archived, created_at, updated_at) VALUES (?,?,?,FALSE,?,?) \
                     ON DUPLICATE KEY UPDATE metadata = VALUES(metadata), archived = FALSE, updated_at = VALUES(updated_at)",
                )
                .bind(&tenant_id)
                .bind(&row_metadata.thread_id)
                .bind(sqlx_core::types::Json(&row_metadata))
                .bind(unix_micros(row_metadata.created_at))
                .bind(unix_micros(row_metadata.updated_at))
                .execute(&pool)
                .await?;
                Ok(())
            })
            .await?;
        Ok(metadata)
    }

    async fn update_thread_metadata(
        &self,
        metadata: ThreadMetadata,
    ) -> anyhow::Result<ThreadMetadata> {
        validate_thread_workspace(&metadata.workspace)?;
        let pool = self.pool.clone();
        let tenant_id = self.tenant_id.clone();
        let row_metadata = metadata.clone();
        self.executor
            .run(async move {
                sqlx_core::query::query::<MySql>("UPDATE roder_sessions SET metadata = ?, updated_at = ? WHERE tenant_id = ? AND thread_id = ? AND archived = FALSE")
                    .bind(sqlx_core::types::Json(&row_metadata))
                    .bind(unix_micros(row_metadata.updated_at))
                    .bind(&tenant_id)
                    .bind(&row_metadata.thread_id)
                    .execute(&pool)
                    .await?;
                Ok(())
            })
            .await?;
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
        let limit = options
            .limit
            .map(|limit| limit as i64)
            .unwrap_or(i64::MAX - 1);
        let pool = self.pool.clone();
        let tenant_id = self.tenant_id.clone();
        let mut threads = self
            .executor
            .run(async move {
                let rows = sqlx_core::query::query::<MySql>("SELECT metadata FROM roder_sessions WHERE tenant_id = ? AND archived = FALSE ORDER BY updated_at DESC LIMIT ? OFFSET ?")
                    .bind(&tenant_id)
                    .bind(limit.saturating_add(1))
                    .bind(offset)
                    .fetch_all(&pool)
                    .await?;
                rows.into_iter()
                    .map(|row| {
                        let json: sqlx_core::types::Json<ThreadMetadata> = row.try_get("metadata")?;
                        Ok(json.0)
                    })
                    .collect::<anyhow::Result<Vec<_>>>()
            })
            .await?;
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
        let pool = self.pool.clone();
        let tenant_id = self.tenant_id.clone();
        let thread_id = thread_id.clone();
        self.executor
            .run(async move {
                let Some(metadata) = load_metadata_on(&pool, &tenant_id, &thread_id).await? else {
                    return Ok(None);
                };
                let event_rows = sqlx_core::query::query::<MySql>("SELECT event FROM roder_session_events WHERE tenant_id = ? AND thread_id = ? ORDER BY seq ASC")
                    .bind(&tenant_id).bind(&thread_id).fetch_all(&pool).await?;
                let events = event_rows
                    .into_iter()
                    .map(|row| {
                        let json: sqlx_core::types::Json<EventEnvelope> = row.try_get("event")?;
                        Ok(json.0)
                    })
                    .collect::<anyhow::Result<Vec<_>>>()?;
                let turns = project_turns_from_events(&thread_id, &events);
                let item_rows = sqlx_core::query::query::<MySql>("SELECT item_event FROM roder_session_item_events WHERE tenant_id = ? AND thread_id = ? ORDER BY seq ASC")
                    .bind(&tenant_id).bind(&thread_id).fetch_all(&pool).await?;
                let item_events = item_rows
                    .into_iter()
                    .map(|row| {
                        let json: sqlx_core::types::Json<ThreadItemEvent> = row.try_get("item_event")?;
                        Ok(json.0)
                    })
                    .collect::<anyhow::Result<Vec<_>>>()?;
                let state_rows = sqlx_core::query::query::<MySql>("SELECT record FROM roder_session_extension_state WHERE tenant_id = ? AND thread_id = ? ORDER BY seq ASC")
                    .bind(&tenant_id).bind(&thread_id).fetch_all(&pool).await?;
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
            })
            .await
    }

    async fn load_thread_metadata(
        &self,
        thread_id: &ThreadId,
    ) -> anyhow::Result<Option<ThreadMetadata>> {
        self.load_metadata(thread_id).await
    }

    async fn archive_thread(&self, thread_id: &ThreadId) -> anyhow::Result<bool> {
        let pool = self.pool.clone();
        let tenant_id = self.tenant_id.clone();
        let thread_id = thread_id.clone();
        self.executor
            .run(async move {
                let result = sqlx_core::query::query::<MySql>("UPDATE roder_sessions SET archived = TRUE, updated_at = ? WHERE tenant_id = ? AND thread_id = ? AND archived = FALSE")
                    .bind(unix_micros_now())
                    .bind(&tenant_id)
                    .bind(&thread_id)
                    .execute(&pool)
                    .await?;
                Ok(result.rows_affected() > 0)
            })
            .await
    }

    async fn append_event(
        &self,
        thread_id: &ThreadId,
        envelope: &EventEnvelope,
    ) -> anyhow::Result<()> {
        let pool = self.pool.clone();
        let tenant_id = self.tenant_id.clone();
        let row_thread_id = thread_id.clone();
        let row_envelope = envelope.clone();
        self.executor
            .run(async move {
                sqlx_core::query::query::<MySql>(
                    "INSERT INTO roder_session_events (tenant_id, thread_id, seq, event, created_at) VALUES (?,?,?,?,?) \
                     ON DUPLICATE KEY UPDATE event = VALUES(event)",
                )
                .bind(&tenant_id)
                .bind(&row_thread_id)
                .bind(row_envelope.seq as i64)
                .bind(sqlx_core::types::Json(&row_envelope))
                .bind(unix_micros_now())
                .execute(&pool)
                .await?;
                Ok(())
            })
            .await?;
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
        let pool = self.pool.clone();
        let tenant_id = self.tenant_id.clone();
        let thread_id = thread_id.clone();
        let item_event = item_event.clone();
        self.executor
            .run(async move {
                sqlx_core::query::query::<MySql>(
                    "INSERT INTO roder_session_item_events (tenant_id, thread_id, seq, item_event, created_at) VALUES (?,?,?,?,?) \
                     ON DUPLICATE KEY UPDATE item_event = VALUES(item_event)",
                )
                .bind(&tenant_id)
                .bind(&thread_id)
                .bind(item_event.seq as i64)
                .bind(sqlx_core::types::Json(&item_event))
                .bind(unix_micros_now())
                .execute(&pool)
                .await?;
                Ok(())
            })
            .await
    }

    async fn append_extension_state(
        &self,
        thread_id: &ThreadId,
        record: &ExtensionStateRecord,
    ) -> anyhow::Result<()> {
        let pool = self.pool.clone();
        let tenant_id = self.tenant_id.clone();
        let thread_id = thread_id.clone();
        let record = record.clone();
        self.executor
            .run(async move {
                sqlx_core::query::query::<MySql>("INSERT INTO roder_session_extension_state (tenant_id, thread_id, record, created_at) VALUES (?,?,?,?)")
                    .bind(&tenant_id)
                    .bind(&thread_id)
                    .bind(sqlx_core::types::Json(&record))
                    .bind(unix_micros_now())
                    .execute(&pool)
                    .await?;
                Ok(())
            })
            .await
    }
}

pub struct MysqlSessionStoreFactory {
    pub config: MysqlSessionConfig,
}

impl ThreadStoreFactory for MysqlSessionStoreFactory {
    fn id(&self) -> ThreadStoreId {
        "mysql-session".to_string()
    }
    fn create(&self) -> Arc<dyn ThreadStore> {
        let store = MysqlSessionStore::connect_blocking(&self.config)
            .unwrap_or_else(|err| panic!("failed to initialize MySQL session store: {}", err));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_config() {
        assert!(MysqlSessionConfig::new("mysql://u:p@localhost/db", "tenant").is_ok());
        assert!(MysqlSessionConfig::new("", "tenant").is_err());
        assert!(MysqlSessionConfig::new("mysql://u:p@localhost/db", "").is_err());
        assert!(MysqlSessionConfig::new("mysql://u:p@localhost/db", "a/b").is_err());
    }

    #[test]
    fn redacts_database_url() {
        assert_eq!(
            redact_database_url("mysql://user:secret@host:3306/db"),
            "mysql://user:<redacted>@host:3306/db"
        );
        assert_eq!(
            redact_database_url("mysql://host:3306/db"),
            "mysql://host:3306/db"
        );
    }
}
