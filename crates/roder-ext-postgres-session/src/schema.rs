use sqlx_core::pool::Pool;
use sqlx_postgres::Postgres;

pub const MIGRATION_VERSION: i32 = 1;

pub async fn migrate(pool: &Pool<Postgres>) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx_core::query::query::<Postgres>(
        r#"CREATE TABLE IF NOT EXISTS roder_session_migrations (
            version INTEGER PRIMARY KEY,
            applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
        )"#,
    )
    .execute(&mut *tx)
    .await?;
    sqlx_core::query::query::<Postgres>(
        r#"CREATE TABLE IF NOT EXISTS roder_sessions (
            tenant_id TEXT NOT NULL,
            thread_id TEXT NOT NULL,
            metadata JSONB NOT NULL,
            archived BOOLEAN NOT NULL DEFAULT FALSE,
            created_at TIMESTAMPTZ NOT NULL,
            updated_at TIMESTAMPTZ NOT NULL,
            PRIMARY KEY (tenant_id, thread_id)
        )"#,
    )
    .execute(&mut *tx)
    .await?;
    sqlx_core::query::query::<Postgres>(
        r#"CREATE TABLE IF NOT EXISTS roder_session_events (
            tenant_id TEXT NOT NULL,
            thread_id TEXT NOT NULL,
            seq BIGINT NOT NULL,
            event JSONB NOT NULL,
            created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
            PRIMARY KEY (tenant_id, thread_id, seq)
        )"#,
    )
    .execute(&mut *tx)
    .await?;
    sqlx_core::query::query::<Postgres>(
        r#"CREATE TABLE IF NOT EXISTS roder_session_item_events (
            tenant_id TEXT NOT NULL,
            thread_id TEXT NOT NULL,
            seq BIGINT NOT NULL,
            item_event JSONB NOT NULL,
            created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
            PRIMARY KEY (tenant_id, thread_id, seq)
        )"#,
    )
    .execute(&mut *tx)
    .await?;
    sqlx_core::query::query::<Postgres>(
        r#"CREATE TABLE IF NOT EXISTS roder_session_extension_state (
            tenant_id TEXT NOT NULL,
            thread_id TEXT NOT NULL,
            seq BIGSERIAL,
            record JSONB NOT NULL,
            created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
            PRIMARY KEY (tenant_id, thread_id, seq)
        )"#,
    )
    .execute(&mut *tx)
    .await?;
    sqlx_core::query::query::<Postgres>(
        r#"CREATE TABLE IF NOT EXISTS roder_context_artifacts (
            tenant_id TEXT NOT NULL,
            thread_id TEXT NOT NULL,
            artifact_id TEXT NOT NULL,
            turn_id TEXT NOT NULL,
            metadata JSONB NOT NULL,
            body BYTEA NOT NULL,
            created_at TIMESTAMPTZ NOT NULL,
            updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
            PRIMARY KEY (tenant_id, thread_id, artifact_id)
        )"#,
    )
    .execute(&mut *tx)
    .await?;
    sqlx_core::query::query::<Postgres>("INSERT INTO roder_session_migrations (version) VALUES ($1) ON CONFLICT (version) DO NOTHING")
        .bind(MIGRATION_VERSION)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}
