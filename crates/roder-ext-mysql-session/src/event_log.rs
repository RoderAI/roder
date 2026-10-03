use roder_api::events::EventEnvelope;
use roder_api::thread::ThreadItemEvent;
use sqlx_core::row::Row;
use sqlx_mysql::{MySql, MySqlConnection};

// MySQL allocates durable ordering; runtime sequence counters can restart.
// Event identity is extracted by the schema's generated unique-key column.
pub(crate) async fn append_event(
    connection: &mut MySqlConnection,
    tenant: &str,
    thread: &str,
    event: &EventEnvelope,
) -> anyhow::Result<bool> {
    let result = sqlx_core::query::query::<MySql>(
        "INSERT INTO roder_session_events (tenant_id, thread_id, event, created_at) VALUES (?,?,?,?)",
    )
    .bind(tenant).bind(thread).bind(sqlx_core::types::Json(event))
    .bind(crate::store::unix_micros_now()).execute(&mut *connection).await;
    match result {
        Ok(_) => {
            if let roder_api::events::RoderEvent::TranscriptItemAppended(event) = &event.event
                && let Some(item) = &event.item
            {
                update_message_metadata(connection, tenant, thread, item).await?;
            }
            Ok(true)
        }
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
    connection: &mut MySqlConnection,
    tenant: &str,
    thread: &str,
    event: &ThreadItemEvent,
) -> anyhow::Result<bool> {
    let result = sqlx_core::query::query::<MySql>(
        "INSERT INTO roder_session_item_events (tenant_id, thread_id, item_event, created_at) VALUES (?,?,?,?)",
    )
    .bind(tenant).bind(thread).bind(sqlx_core::types::Json(event))
    .bind(crate::store::unix_micros_now()).execute(&mut *connection).await;
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

async fn update_message_metadata(
    connection: &mut MySqlConnection,
    tenant: &str,
    thread: &str,
    item: &roder_api::transcript::TranscriptItem,
) -> anyhow::Result<()> {
    use roder_api::thread::ThreadMetadata;
    use roder_api::transcript::TranscriptItem;
    if !matches!(
        item,
        TranscriptItem::UserMessage(_) | TranscriptItem::AssistantMessage(_)
    ) {
        return Ok(());
    }
    let row = sqlx_core::query::query::<MySql>("SELECT metadata FROM roder_sessions WHERE tenant_id = ? AND thread_id = ? AND archived = FALSE")
        .bind(tenant).bind(thread).fetch_optional(&mut *connection).await?
        .ok_or_else(|| anyhow::anyhow!("thread metadata missing for {thread}"))?;
    let mut metadata = row
        .try_get::<sqlx_core::types::Json<ThreadMetadata>, _>("metadata")?
        .0;
    metadata.updated_at = time::OffsetDateTime::now_utc();
    metadata.message_count = metadata.message_count.saturating_add(1);
    if metadata
        .title
        .as_ref()
        .is_none_or(|title| title.trim().is_empty())
        && let TranscriptItem::UserMessage(message) = item
    {
        metadata.title = crate::store::title_from_user_text(&message.text);
    }
    sqlx_core::query::query::<MySql>("UPDATE roder_sessions SET metadata = ?, updated_at = ? WHERE tenant_id = ? AND thread_id = ?")
        .bind(sqlx_core::types::Json(&metadata)).bind(crate::store::unix_micros(metadata.updated_at))
        .bind(tenant).bind(thread).execute(connection).await?;
    Ok(())
}
