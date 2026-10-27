use anyhow::Context;
use roder_api::thread::{
    ThreadItem, ThreadItemDelta, ThreadItemEvent, ThreadItemEventKind, project_thread_item_events,
};
use rusqlite::{Connection, Transaction, params};
use sha2::{Digest, Sha256};

/// Materialize visibility at checkpoints so warm pages seek only visible rows,
/// even if years of saved/projected duplicates exist above their cursor.
pub(super) fn refresh_visibility(
    tx: &Transaction<'_>,
    turn: &str,
    old: &str,
    new: &str,
) -> anyhow::Result<()> {
    tx.execute("UPDATE items SET visible= \
        (SELECT COUNT(*) FROM items s WHERE s.turn_id=items.turn_id AND s.source=0 \
            AND s.signature=items.signature AND s.key<=items.key) > \
        (SELECT COUNT(*) FROM items p WHERE p.turn_id=items.turn_id AND p.source=1 AND p.signature=items.signature) \
        WHERE turn_id=?1 AND source=0 AND signature IN (?2,?3)",params![turn,old,new])?;
    Ok(())
}

/// Recognize saved equivalents without presentation ID/status differences.
/// Computed at checkpoints only, never for streamed deltas.
pub(super) fn of(item: &ThreadItem) -> anyhow::Result<String> {
    if let ThreadItem::ToolExecution { tool_call_id, .. } = item {
        return Ok(format!("tool:{tool_call_id}"));
    }
    let mut value = serde_json::to_value(item)?;
    if let Some(object) = value.as_object_mut() {
        object.remove("id");
        object.remove("status");
    }
    Ok(Sha256::digest(serde_json::to_vec(&value)?)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

pub(super) fn set_id(item: &mut ThreadItem, normalized: String) {
    match item {
        ThreadItem::UserMessage { id, .. }
        | ThreadItem::AgentMessage { id, .. }
        | ThreadItem::Reasoning { id, .. }
        | ThreadItem::ToolExecution { id, .. }
        | ThreadItem::RoutingDecision { id, .. }
        | ThreadItem::Compaction { id, .. }
        | ThreadItem::Error { id, .. }
        | ThreadItem::Raw { id, .. } => *id = normalized,
    }
}

/// Upper bound without rebuilding growing text on every delta. Sparse reasoning
/// array indices may add empty JSON string slots; account for those as well.
pub(super) fn event_budget(event: &ThreadItemEvent, serialized: usize) -> usize {
    let slots = match &event.event {
        ThreadItemEventKind::ItemDelta {
            delta: ThreadItemDelta::ReasoningText { content_index, .. },
            ..
        } => *content_index,
        ThreadItemEventKind::ItemDelta {
            delta:
                ThreadItemDelta::ReasoningSummaryText { summary_index, .. }
                | ThreadItemDelta::ReasoningSummaryPartAdded { summary_index },
            ..
        } => *summary_index,
        _ => 0,
    };
    serialized
        .saturating_add(128)
        .saturating_add(slots.saturating_mul(3))
}

pub(super) fn project_item(db: &Connection, key: i64) -> anyhow::Result<ThreadItem> {
    let mut statement = db.prepare("SELECT payload FROM records WHERE item_key=?1 ORDER BY seq")?;
    let mut rows = statement.query([key])?;
    let mut events = Vec::new();
    while let Some(row) = rows.next()? {
        events.push(serde_json::from_str::<ThreadItemEvent>(
            &row.get::<_, String>(0)?,
        )?);
    }
    project_thread_item_events(&events)
        .into_iter()
        .next()
        .and_then(|t| t.items.into_iter().next())
        .context("indexed item has no projection")
}
