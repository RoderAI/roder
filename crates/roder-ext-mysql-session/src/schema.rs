use sqlx_core::pool::Pool;
use sqlx_mysql::MySql;

pub const MIGRATION_VERSION: i32 = 3;

/// Key columns use VARCHAR(191) so composite primary keys stay within
/// InnoDB's index size limits under utf8mb4. Timestamps are unix
/// microseconds (BIGINT).
pub async fn migrate(pool: &Pool<MySql>) -> anyhow::Result<()> {
    let statements = [
        r#"CREATE TABLE IF NOT EXISTS roder_runtime_owners (
            tenant_id VARBINARY(191) PRIMARY KEY,
            owner_id CHAR(36) CHARACTER SET ascii COLLATE ascii_bin NULL,
            generation BIGINT UNSIGNED NOT NULL,
            endpoint VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NULL,
            expires_at BIGINT NOT NULL
        )"#,
        r#"CREATE TABLE IF NOT EXISTS roder_session_migrations (
            version INT PRIMARY KEY,
            applied_at BIGINT NOT NULL
        )"#,
        r#"CREATE TABLE IF NOT EXISTS roder_sessions (
            tenant_id VARCHAR(191) NOT NULL,
            thread_id VARCHAR(191) NOT NULL,
            metadata JSON NOT NULL,
            archived BOOLEAN NOT NULL DEFAULT FALSE,
            created_at BIGINT NOT NULL,
            updated_at BIGINT NOT NULL,
            PRIMARY KEY (tenant_id, thread_id),
            KEY idx_roder_sessions_updated (tenant_id, archived, updated_at)
        )"#,
        r#"CREATE TABLE IF NOT EXISTS roder_session_events (
            tenant_id VARCHAR(191) NOT NULL,
            thread_id VARCHAR(191) NOT NULL,
            seq BIGINT NOT NULL,
            event JSON NOT NULL,
            created_at BIGINT NOT NULL,
            PRIMARY KEY (tenant_id, thread_id, seq)
        )"#,
        r#"CREATE TABLE IF NOT EXISTS roder_session_item_events (
            tenant_id VARCHAR(191) NOT NULL,
            thread_id VARCHAR(191) NOT NULL,
            seq BIGINT NOT NULL,
            item_event JSON NOT NULL,
            created_at BIGINT NOT NULL,
            PRIMARY KEY (tenant_id, thread_id, seq)
        )"#,
        r#"CREATE TABLE IF NOT EXISTS roder_session_extension_state (
            tenant_id VARCHAR(191) NOT NULL,
            thread_id VARCHAR(191) NOT NULL,
            seq BIGINT NOT NULL AUTO_INCREMENT,
            record JSON NOT NULL,
            created_at BIGINT NOT NULL,
            PRIMARY KEY (tenant_id, thread_id, seq),
            KEY idx_roder_session_extension_state_seq (seq)
        )"#,
        r#"CREATE TABLE IF NOT EXISTS roder_context_artifacts (
            tenant_id VARCHAR(191) NOT NULL,
            thread_id VARCHAR(191) NOT NULL,
            artifact_id VARCHAR(191) NOT NULL,
            turn_id VARCHAR(191) NOT NULL,
            metadata JSON NOT NULL,
            body LONGBLOB NOT NULL,
            created_at BIGINT NOT NULL,
            updated_at BIGINT NOT NULL,
            PRIMARY KEY (tenant_id, thread_id, artifact_id)
        )"#,
    ];
    for statement in statements {
        sqlx_core::query::query::<MySql>(statement)
            .execute(pool)
            .await?;
    }
    migrate_event_ordering(pool).await?;
    sqlx_core::query::query::<MySql>(
        "INSERT IGNORE INTO roder_session_migrations (version, applied_at) VALUES (?, ?)",
    )
    .bind(MIGRATION_VERSION)
    .bind(crate::store::unix_micros_now())
    .execute(pool)
    .await?;
    Ok(())
}

/// Runtime counters restart and are not durable identities. Existing payloads stay intact.
async fn migrate_event_ordering(pool: &Pool<MySql>) -> anyhow::Result<()> {
    // Detach so cancellation closes this connection and releases its advisory lock.
    let mut connection = pool.acquire().await?.detach();
    let locked: Option<i64> = sqlx_core::query_scalar::query_scalar::<MySql, Option<i64>>(
        "SELECT GET_LOCK(CONCAT('roder-v2-', MD5(DATABASE())), 60)",
    )
    .fetch_one(&mut connection)
    .await?;
    anyhow::ensure!(
        locked == Some(1),
        "Timed out acquiring session schema migration lock"
    );
    for (table, statement) in [
        (
            "roder_session_events",
            "ALTER TABLE roder_session_events ADD KEY idx_durable_seq (seq), MODIFY seq BIGINT NOT NULL AUTO_INCREMENT, ADD COLUMN event_id BINARY(32) GENERATED ALWAYS AS (UNHEX(SHA2(JSON_UNQUOTE(JSON_EXTRACT(event, '$.event_id')), 256))) STORED, ADD UNIQUE KEY idx_event_identity (tenant_id, thread_id, event_id)",
        ),
        (
            "roder_session_item_events",
            "ALTER TABLE roder_session_item_events ADD KEY idx_durable_seq (seq), MODIFY seq BIGINT NOT NULL AUTO_INCREMENT, ADD COLUMN event_id BINARY(32) GENERATED ALWAYS AS (UNHEX(SHA2(JSON_UNQUOTE(JSON_EXTRACT(item_event, '$.eventId')), 256))) STORED, ADD UNIQUE KEY idx_event_identity (tenant_id, thread_id, event_id)",
        ),
    ] {
        let migrated: i64 = sqlx_core::query_scalar::query_scalar::<MySql, i64>(
            "SELECT COUNT(*) FROM information_schema.columns WHERE table_schema = DATABASE() AND table_name = ? AND column_name = 'event_id'",
        ).bind(table).fetch_one(&mut connection).await?;
        if migrated == 0 {
            // One atomic DDL per table; reconnecting resumes after either completed table.
            sqlx_core::query::query::<MySql>(statement)
                .execute(&mut connection)
                .await?;
        }
    }
    Ok(())
}
