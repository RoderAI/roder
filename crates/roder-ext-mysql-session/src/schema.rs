use sqlx_core::pool::Pool;
use sqlx_mysql::MySql;

pub const MIGRATION_VERSION: i32 = 1;

/// Key columns use VARCHAR(191) so composite primary keys stay within
/// InnoDB's index size limits under utf8mb4. Timestamps are unix
/// microseconds (BIGINT).
pub async fn migrate(pool: &Pool<MySql>) -> anyhow::Result<()> {
    let statements = [
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
    sqlx_core::query::query::<MySql>(
        "INSERT IGNORE INTO roder_session_migrations (version, applied_at) VALUES (?, ?)",
    )
    .bind(MIGRATION_VERSION)
    .bind(crate::store::unix_micros_now())
    .execute(pool)
    .await?;
    Ok(())
}
