use roder_api::events::EventEnvelope;
use roder_api::thread::ThreadItemEvent;
use sqlx_core::pool::Pool;
use sqlx_mysql::MySql;

// MySQL allocates durable ordering; runtime sequence counters can restart.
// Event identity is extracted by the schema's generated unique-key column.
pub(crate) async fn append_event(
    pool: &Pool<MySql>,
    tenant: &str,
    thread: &str,
    event: &EventEnvelope,
) -> anyhow::Result<()> {
    sqlx_core::query::query::<MySql>(
        "INSERT INTO roder_session_events (tenant_id, thread_id, event, created_at) VALUES (?,?,?,?) \
         ON DUPLICATE KEY UPDATE event = event",
    )
    .bind(tenant).bind(thread).bind(sqlx_core::types::Json(event))
    .bind(crate::store::unix_micros_now()).execute(pool).await?;
    Ok(())
}

pub(crate) async fn append_item_event(
    pool: &Pool<MySql>,
    tenant: &str,
    thread: &str,
    event: &ThreadItemEvent,
) -> anyhow::Result<()> {
    sqlx_core::query::query::<MySql>(
        "INSERT INTO roder_session_item_events (tenant_id, thread_id, item_event, created_at) VALUES (?,?,?,?) \
         ON DUPLICATE KEY UPDATE item_event = item_event",
    )
    .bind(tenant).bind(thread).bind(sqlx_core::types::Json(event))
    .bind(crate::store::unix_micros_now()).execute(pool).await?;
    Ok(())
}
