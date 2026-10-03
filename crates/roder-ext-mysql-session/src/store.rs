use crate::{MysqlSessionConfig, validate_tenant_id};
use std::sync::Arc;

use anyhow::Context;
use roder_api::artifacts::ContextArtifactStore;
use roder_api::events::{EventEnvelope, ThreadId};
use roder_api::extension::ThreadStoreId;
use roder_api::extension_state::ExtensionStateRecord;
use roder_api::thread::{
    ThreadItemEvent, ThreadListOptions, ThreadListPage, ThreadMetadata, ThreadSnapshot,
    ThreadStore, ThreadStoreFactory, project_turns_from_events, validate_thread_workspace,
};
use sqlx_core::pool::Pool;
use sqlx_core::row::Row;
use sqlx_mysql::MySql;
use time::OffsetDateTime;

use crate::artifacts::MysqlArtifactStore;
use crate::executor::DbExecutor;
use crate::schema;

pub(crate) fn unix_micros(timestamp: OffsetDateTime) -> i64 {
    (timestamp.unix_timestamp_nanos() / 1_000) as i64
}

pub(crate) fn unix_micros_now() -> i64 {
    unix_micros(OffsetDateTime::now_utc())
}

#[derive(Clone)]
pub struct MysqlSessionStore {
    pub(crate) executor: Arc<DbExecutor>,
    pub(crate) pool: Pool<MySql>,
    tenant_id: String,
    pub(crate) owner: Option<crate::ownership::RuntimeOwnerLease>,
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
            owner: None,
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
            owner: None,
        })
    }

    /// Derives a tenant-scoped handle sharing this store's pool and runtime,
    /// mirroring the PostgreSQL store's hosted-tenancy contract.
    pub fn for_tenant(&self, tenant_id: &str) -> anyhow::Result<Self> {
        let tenant_id = validate_tenant_id(tenant_id)?;
        anyhow::ensure!(self.owner.is_none() || tenant_id == self.tenant_id, "cannot rescope an owned session store");
        Ok(Self {
            executor: self.executor.clone(),
            pool: self.pool.clone(),
            tenant_id,
            owner: self.owner.clone(),
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
            owner: self.owner.clone(),
        }))
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

async fn load_extension_state_records_on(
    pool: &Pool<MySql>,
    tenant_id: &str,
    thread_id: &str,
) -> anyhow::Result<Vec<ExtensionStateRecord>> {
    let state_rows = sqlx_core::query::query::<MySql>("SELECT record FROM roder_session_extension_state WHERE tenant_id = ? AND thread_id = ? ORDER BY seq ASC")
        .bind(tenant_id).bind(thread_id).fetch_all(pool).await?;
    state_rows
        .into_iter()
        .map(|row| {
            let json: sqlx_core::types::Json<ExtensionStateRecord> = row.try_get("record")?;
            Ok(json.0)
        })
        .collect::<anyhow::Result<Vec<_>>>()
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
        let owner = self.owner.clone();
        let tenant_id = self.tenant_id.clone();
        let row_metadata = metadata.clone();
        self.executor
            .run(async move {
                let mut tx = crate::write_guard::begin(&pool, &tenant_id, owner.as_ref()).await?;
                sqlx_core::query::query::<MySql>(
                    "INSERT INTO roder_sessions (tenant_id, thread_id, metadata, archived, created_at, updated_at) VALUES (?,?,?,FALSE,?,?) \
                     ON DUPLICATE KEY UPDATE metadata = VALUES(metadata), archived = FALSE, updated_at = VALUES(updated_at)",
                )
                .bind(&tenant_id)
                .bind(&row_metadata.thread_id)
                .bind(sqlx_core::types::Json(&row_metadata))
                .bind(unix_micros(row_metadata.created_at))
                .bind(unix_micros(row_metadata.updated_at))
                .execute(&mut *tx)
                .await?;
                tx.commit().await?;
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
        let owner = self.owner.clone();
        let tenant_id = self.tenant_id.clone();
        let row_metadata = metadata.clone();
        self.executor
            .run(async move {
                let mut tx = crate::write_guard::begin(&pool, &tenant_id, owner.as_ref()).await?;
                sqlx_core::query::query::<MySql>("UPDATE roder_sessions SET metadata = ?, updated_at = ? WHERE tenant_id = ? AND thread_id = ? AND archived = FALSE")
                    .bind(sqlx_core::types::Json(&row_metadata))
                    .bind(unix_micros(row_metadata.updated_at))
                    .bind(&tenant_id)
                    .bind(&row_metadata.thread_id)
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;
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
                let event_rows = sqlx_core::query::query::<MySql>("SELECT seq, event FROM roder_session_events WHERE tenant_id = ? AND thread_id = ? ORDER BY seq ASC")
                    .bind(&tenant_id).bind(&thread_id).fetch_all(&pool).await?;
                let events = event_rows
                    .into_iter()
                    .map(|row| {
                        let mut json: sqlx_core::types::Json<EventEnvelope> = row.try_get("event")?;
                        json.0.seq = u64::try_from(row.try_get::<i64, _>("seq")?)?;
                        Ok(json.0)
                    })
                    .collect::<anyhow::Result<Vec<_>>>()?;
                let turns = project_turns_from_events(&thread_id, &events);
                let item_rows = sqlx_core::query::query::<MySql>("SELECT seq, item_event FROM roder_session_item_events WHERE tenant_id = ? AND thread_id = ? ORDER BY seq ASC")
                    .bind(&tenant_id).bind(&thread_id).fetch_all(&pool).await?;
                let item_events = item_rows
                    .into_iter()
                    .map(|row| {
                        let mut json: sqlx_core::types::Json<ThreadItemEvent> = row.try_get("item_event")?;
                        json.0.seq = u64::try_from(row.try_get::<i64, _>("seq")?)?;
                        Ok(json.0)
                    })
                    .collect::<anyhow::Result<Vec<_>>>()?;
                let extension_states =
                    load_extension_state_records_on(&pool, &tenant_id, &thread_id).await?;
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

    async fn load_extension_states(
        &self,
        thread_id: &ThreadId,
    ) -> anyhow::Result<Vec<ExtensionStateRecord>> {
        let pool = self.pool.clone();
        let tenant_id = self.tenant_id.clone();
        let thread_id = thread_id.clone();
        self.executor
            .run(
                async move { load_extension_state_records_on(&pool, &tenant_id, &thread_id).await },
            )
            .await
    }

    async fn archive_thread(&self, thread_id: &ThreadId) -> anyhow::Result<bool> {
        let pool = self.pool.clone();
        let owner = self.owner.clone();
        let tenant_id = self.tenant_id.clone();
        let thread_id = thread_id.clone();
        self.executor
            .run(async move {
                let mut tx = crate::write_guard::begin(&pool, &tenant_id, owner.as_ref()).await?;
                let result = sqlx_core::query::query::<MySql>("UPDATE roder_sessions SET archived = TRUE, updated_at = ? WHERE tenant_id = ? AND thread_id = ? AND archived = FALSE")
                    .bind(unix_micros_now())
                    .bind(&tenant_id)
                    .bind(&thread_id)
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;
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
        let owner = self.owner.clone();
        let tenant_id = self.tenant_id.clone();
        let row_thread_id = thread_id.clone();
        let row_envelope = envelope.clone();
        self.executor
            .run(async move {
                let mut tx = crate::write_guard::begin(&pool, &tenant_id, owner.as_ref()).await?;
                crate::event_log::append_event(&mut tx, &tenant_id, &row_thread_id, &row_envelope).await?;
                tx.commit().await?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    async fn append_item_event(
        &self,
        thread_id: &ThreadId,
        item_event: &ThreadItemEvent,
    ) -> anyhow::Result<()> {
        let pool = self.pool.clone();
        let owner = self.owner.clone();
        let tenant_id = self.tenant_id.clone();
        let thread_id = thread_id.clone();
        let item_event = item_event.clone();
        self.executor
            .run(async move {
                let mut tx = crate::write_guard::begin(&pool, &tenant_id, owner.as_ref()).await?;
                crate::event_log::append_item_event(&mut tx, &tenant_id, &thread_id, &item_event).await?;
                tx.commit().await?;
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
        let owner = self.owner.clone();
        let tenant_id = self.tenant_id.clone();
        let thread_id = thread_id.clone();
        let record = record.clone();
        self.executor
            .run(async move {
                let mut tx = crate::write_guard::begin(&pool, &tenant_id, owner.as_ref()).await?;
                sqlx_core::query::query::<MySql>("INSERT INTO roder_session_extension_state (tenant_id, thread_id, record, created_at) VALUES (?,?,?,?)")
                    .bind(&tenant_id)
                    .bind(&thread_id)
                    .bind(sqlx_core::types::Json(&record))
                    .bind(unix_micros_now())
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;
                Ok(())
            })
            .await
    }
}

impl ThreadStoreFactory for MysqlSessionStore {
    fn id(&self) -> ThreadStoreId { "mysql-session".to_string() }
    fn create(&self) -> Arc<dyn ThreadStore> { Arc::new(self.clone()) }
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

pub(crate) fn title_from_user_text(text: &str) -> Option<String> {
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
