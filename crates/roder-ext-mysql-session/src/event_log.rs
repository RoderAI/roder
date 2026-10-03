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
) -> anyhow::Result<bool> {
    let result = sqlx_core::query::query::<MySql>(
        "INSERT INTO roder_session_events (tenant_id, thread_id, event, created_at) VALUES (?,?,?,?)",
    )
    .bind(tenant).bind(thread).bind(sqlx_core::types::Json(event))
    .bind(crate::store::unix_micros_now()).execute(pool).await;
    match result {
        Ok(_) => Ok(true),
        Err(error)
            if error
                .as_database_error()
                .is_some_and(|error| error.is_unique_violation()) =>
        {
            Ok(false)
        }
        Err(error) => Err(error.into()),
    }
}

pub(crate) async fn append_item_event(
    pool: &Pool<MySql>,
    tenant: &str,
    thread: &str,
    event: &ThreadItemEvent,
) -> anyhow::Result<bool> {
    let result = sqlx_core::query::query::<MySql>(
        "INSERT INTO roder_session_item_events (tenant_id, thread_id, item_event, created_at) VALUES (?,?,?,?)",
    )
    .bind(tenant).bind(thread).bind(sqlx_core::types::Json(event))
    .bind(crate::store::unix_micros_now()).execute(pool).await;
    match result {
        Ok(_) => Ok(true),
        Err(error)
            if error
                .as_database_error()
                .is_some_and(|error| error.is_unique_violation()) =>
        {
            Ok(false)
        }
        Err(error) => Err(error.into()),
    }
}
