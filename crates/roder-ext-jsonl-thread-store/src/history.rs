//! Derived, disposable history index. A cold rebuild streams each committed JSONL
//! record once; warm reads and appends only scan the suffix after durable offsets.
//! Payload checkpoints/deltas are indexed by logical item, never by whole thread.
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, ensure};
use roder_api::events::{EventEnvelope, RoderEvent, ThreadId};
use roder_api::thread::{
    ThreadHistoryPage, ThreadHistoryReadOptions, ThreadHistoryTurn, ThreadHistoryTurnStatus,
    ThreadItem, ThreadItemEvent, ThreadItemEventKind, ThreadMetadata,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use time::OffsetDateTime;

mod signature;
use signature::project_item;
mod cursor;
#[cfg(test)]
mod tests;
mod transcript;
use cursor::{Cursor, decode, encode};

// Keep projected identity through completion, supplementing saved-only content.
const VISIBLE: &str = "i.visible=1";

pub(crate) async fn sync(dir: PathBuf) -> anyhow::Result<()> {
    tokio::task::spawn_blocking(move || {
        let mut db = open(&dir)?;
        synchronize(&mut db, &dir)
    })
    .await?
}

pub(crate) async fn read(
    dir: PathBuf,
    thread_id: ThreadId,
    options: ThreadHistoryReadOptions,
) -> anyhow::Result<Option<ThreadHistoryPage>> {
    validate_thread_id(&thread_id)?;
    tokio::task::spawn_blocking(move || read_sync(&dir, &thread_id, options)).await?
}

fn validate_thread_id(thread: &str) -> anyhow::Result<()> {
    ensure!(
        !thread.is_empty()
            && thread != "."
            && thread != ".."
            && !thread.contains(['/', '\\', '\0']),
        "invalid history thread id"
    );
    Ok(())
}

fn open(dir: &Path) -> anyhow::Result<Connection> {
    let db = Connection::open(dir.join("history.sqlite"))?;
    db.busy_timeout(Duration::from_secs(30))?;
    db.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE TABLE IF NOT EXISTS offsets(source INTEGER PRIMARY KEY, pos INTEGER NOT NULL);
         CREATE TABLE IF NOT EXISTS turns(
           id TEXT PRIMARY KEY, ns INTEGER NOT NULL, payload TEXT NOT NULL,
           terminal INTEGER NOT NULL DEFAULT 0);
         CREATE INDEX IF NOT EXISTS history_turn_order ON turns(ns,id);
         CREATE TABLE IF NOT EXISTS items(
           key INTEGER PRIMARY KEY AUTOINCREMENT, turn_id TEXT NOT NULL,
           source INTEGER NOT NULL, item_id TEXT NOT NULL, raw_id TEXT NOT NULL,
           completed INTEGER NOT NULL DEFAULT 0, visible INTEGER NOT NULL DEFAULT 1,
           bytes INTEGER NOT NULL DEFAULT 0,
           signature TEXT NOT NULL,
           turn_ns INTEGER NOT NULL, ns INTEGER NOT NULL,
           UNIQUE(turn_id,source,item_id));
         CREATE INDEX IF NOT EXISTS history_turn_items ON items(turn_id,source,key);
         CREATE INDEX IF NOT EXISTS history_item_order ON items(turn_ns,turn_id,ns,source,key)
           WHERE visible=1;
         CREATE INDEX IF NOT EXISTS history_signatures ON items(turn_id,source,signature);
         CREATE INDEX IF NOT EXISTS history_raw_ids ON items(turn_id,source,raw_id,key);
         CREATE TABLE IF NOT EXISTS records(
           seq INTEGER PRIMARY KEY AUTOINCREMENT, item_key INTEGER NOT NULL,
           payload TEXT NOT NULL);
         CREATE INDEX IF NOT EXISTS history_item_records ON records(item_key,seq);",
    )?;
    Ok(db)
}

fn synchronize(db: &mut Connection, dir: &Path) -> anyhow::Result<()> {
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    for (source, name) in [(0, "events.jsonl"), (1, "item_events.jsonl")] {
        let path = dir.join(name);
        let mut reader = match File::open(&path) {
            Ok(file) => BufReader::new(file),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err.into()),
        };
        let mut pos = u64::try_from(
            tx.query_row("SELECT pos FROM offsets WHERE source=?1", [source], |r| {
                r.get::<_, i64>(0)
            })
            .optional()?
            .unwrap_or(0),
        )?;
        ensure!(
            reader.get_ref().metadata()?.len() >= pos,
            "history source was truncated"
        );
        reader.seek(SeekFrom::Start(pos))?;
        let mut line = Vec::new();
        loop {
            line.clear();
            let count = reader.read_until(b'\n', &mut line)?;
            // A writer may have only published a prefix. Never commit its offset.
            if count == 0 || line.last() != Some(&b'\n') {
                break;
            }
            if !line.iter().all(u8::is_ascii_whitespace) {
                if source == 0 {
                    for (index, envelope) in serde_json::Deserializer::from_slice(&line)
                        .into_iter::<EventEnvelope>()
                        .enumerate()
                    {
                        let envelope = envelope
                            .with_context(|| format!("parse {} at {pos}", path.display()))?;
                        index_event(&tx, &envelope, pos.saturating_add(index as u64))?;
                    }
                } else {
                    let event: ThreadItemEvent = serde_json::from_slice(&line)
                        .with_context(|| format!("parse {} at {pos}", path.display()))?;
                    ensure_turn(&tx, &event.turn_id, event.timestamp)?;
                    index_item(&tx, source, &event)?;
                }
            }
            pos += count as u64;
        }
        tx.execute(
            "INSERT INTO offsets VALUES(?1,?2) ON CONFLICT(source) DO UPDATE SET pos=excluded.pos",
            params![source, i64::try_from(pos)?],
        )?;
    }
    tx.commit()?;
    Ok(())
}

fn ensure_turn(tx: &Transaction<'_>, id: &str, timestamp: OffsetDateTime) -> anyhow::Result<()> {
    let existing: Option<String> = tx
        .query_row("SELECT payload FROM turns WHERE id=?1", [id], |r| r.get(0))
        .optional()?;
    if let Some(existing) = existing {
        let mut turn: ThreadHistoryTurn = serde_json::from_str(&existing)?;
        if timestamp < turn.created_at {
            turn.created_at = timestamp;
            tx.execute(
                "UPDATE turns SET ns=?2,payload=?3 WHERE id=?1",
                params![
                    id,
                    i64::try_from(timestamp.unix_timestamp_nanos())?,
                    serde_json::to_string(&turn)?
                ],
            )?;
            tx.execute(
                "UPDATE items SET turn_ns=?2 WHERE turn_id=?1",
                params![id, i64::try_from(timestamp.unix_timestamp_nanos())?],
            )?;
        }
        return Ok(());
    }
    let turn = ThreadHistoryTurn {
        turn_id: id.into(),
        created_at: timestamp,
        completed_at: None,
        status: ThreadHistoryTurnStatus::InProgress,
        error: None,
        usage: None,
        finish_reason: None,
        items: Vec::new(),
    };
    tx.execute(
        "INSERT OR IGNORE INTO turns(id,ns,payload) VALUES(?1,?2,?3)",
        params![
            id,
            i64::try_from(timestamp.unix_timestamp_nanos())?,
            serde_json::to_string(&turn)?
        ],
    )?;
    Ok(())
}

fn index_event(tx: &Transaction<'_>, envelope: &EventEnvelope, offset: u64) -> anyhow::Result<()> {
    let (id, timestamp) = match &envelope.event {
        RoderEvent::TurnStarted(e) => (&e.turn_id, e.timestamp),
        RoderEvent::TranscriptItemAppended(e) => (&e.turn_id, e.timestamp),
        RoderEvent::TurnCompleted(e) => (&e.turn_id, e.timestamp),
        RoderEvent::TurnFailed(e) => (&e.turn_id, e.timestamp),
        RoderEvent::TurnInterrupted(e) => (&e.turn_id, e.timestamp),
        _ => return Ok(()),
    };
    ensure_turn(tx, id, timestamp)?;
    if let RoderEvent::TranscriptItemAppended(e) = &envelope.event {
        if let Some(item) = &e.item {
            let item =
                transcript::convert(id, e.item_index.map(|i| i as u64).unwrap_or(offset), item);
            let event = ThreadItemEvent {
                seq: offset,
                event_id: format!("transcript-{offset}"),
                thread_id: e.thread_id.clone(),
                turn_id: id.clone(),
                timestamp,
                event: if matches!(
                    &item,
                    ThreadItem::ToolExecution {
                        status: roder_api::thread::ThreadItemStatus::InProgress,
                        ..
                    }
                ) {
                    ThreadItemEventKind::ItemStarted { item }
                } else {
                    ThreadItemEventKind::ItemCompleted { item }
                },
            };
            index_item(tx, 0, &event)?;
        }
        return Ok(());
    }
    let data: String = tx.query_row("SELECT payload FROM turns WHERE id=?1", [id], |r| r.get(0))?;
    let mut turn: ThreadHistoryTurn = serde_json::from_str(&data)?;
    match &envelope.event {
        RoderEvent::TurnStarted(e) => turn.created_at = e.timestamp,
        RoderEvent::TurnCompleted(e) => {
            turn.completed_at = Some(e.timestamp);
            turn.status = ThreadHistoryTurnStatus::Completed;
            turn.usage = e.usage.clone();
            turn.finish_reason = e.finish_reason.clone();
        }
        RoderEvent::TurnFailed(e) => {
            turn.completed_at = Some(e.timestamp);
            turn.status = ThreadHistoryTurnStatus::Failed;
            turn.error = Some(e.error.clone());
            turn.usage = e.usage.clone();
        }
        RoderEvent::TurnInterrupted(e) => {
            turn.completed_at = Some(e.timestamp);
            turn.status = ThreadHistoryTurnStatus::Interrupted;
            turn.usage = e.usage.clone();
        }
        _ => {}
    }
    tx.execute(
        "UPDATE turns SET ns=?2,payload=?3,terminal=?4 WHERE id=?1",
        params![
            id,
            i64::try_from(turn.created_at.unix_timestamp_nanos())?,
            serde_json::to_string(&turn)?,
            i64::from(turn.completed_at.is_some())
        ],
    )?;
    Ok(())
}

fn index_item(tx: &Transaction<'_>, source: i64, event: &ThreadItemEvent) -> anyhow::Result<()> {
    let id = match &event.event {
        ThreadItemEventKind::ItemStarted { item } | ThreadItemEventKind::ItemCompleted { item } => {
            item.id()
        }
        ThreadItemEventKind::ItemDelta { item_id, .. } => item_id,
    };
    let signature = match &event.event {
        ThreadItemEventKind::ItemStarted { item } | ThreadItemEventKind::ItemCompleted { item } => {
            signature::of(item)?
        }
        _ => format!("delta-{id}"),
    };
    let existing:Option<(i64,bool,String)> = tx.query_row(
        "SELECT key,completed,signature FROM items WHERE turn_id=?1 AND source=?2 AND raw_id=?3 ORDER BY key DESC LIMIT 1",
        params![event.turn_id,source,id], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    let starts_new = existing.as_ref().is_some_and(|(_, completed, old)| {
        *completed
            && (matches!(event.event, ThreadItemEventKind::ItemStarted { .. })
                || (matches!(&event.event,ThreadItemEventKind::ItemCompleted {item}
            if !matches!(item,ThreadItem::ToolExecution {..}))
                    && old != &signature))
    });
    let key = if let Some((key, _, _)) = existing.filter(|_| !starts_new) {
        key
    } else {
        let mut normalized = id.to_string();
        let mut duplicate = 2;
        while tx
            .query_row(
                "SELECT 1 FROM items WHERE turn_id=?1 AND source=?2 AND item_id=?3",
                params![event.turn_id, source, normalized],
                |_| Ok(()),
            )
            .optional()?
            .is_some()
        {
            normalized = format!("{id}::duplicate-{duplicate}");
            duplicate += 1;
        }
        tx.execute(
            "INSERT INTO items(turn_id,source,item_id,raw_id,signature,turn_ns,ns) \
             SELECT ?1,?2,?3,?4,?5,ns,?6 FROM turns WHERE id=?1",
            params![
                event.turn_id,
                source,
                normalized,
                id,
                signature,
                i64::try_from(event.timestamp.unix_timestamp_nanos())?
            ],
        )?;
        tx.last_insert_rowid()
    };
    let payload = serde_json::to_string(event)?;
    tx.execute(
        "INSERT INTO records(item_key,payload) VALUES(?1,?2)",
        params![key, payload],
    )?;
    tx.execute(
        "UPDATE items SET bytes=bytes+?2 WHERE key=?1",
        params![
            key,
            i64::try_from(signature::event_budget(event, payload.len()))?
        ],
    )?;
    // Compact once at completion, not on every streamed token. The shared projector
    // preserves tool inputs and interleaved delta semantics.
    if matches!(event.event, ThreadItemEventKind::ItemCompleted { .. }) {
        let item = project_item(tx, key)?;
        let old: String = tx.query_row("SELECT signature FROM items WHERE key=?1", [key], |r| {
            r.get(0)
        })?;
        let new = signature::of(&item)?;
        tx.execute(
            "UPDATE items SET signature=?2,completed=1,bytes=?3 WHERE key=?1",
            params![
                key,
                new,
                i64::try_from(serde_json::to_vec(&item)?.len().saturating_add(128))?
            ],
        )?;
        signature::refresh_visibility(tx, &event.turn_id, &old, &new)?;
        let mut checkpoint = event.clone();
        checkpoint.event = ThreadItemEventKind::ItemCompleted { item };
        tx.execute("DELETE FROM records WHERE item_key=?1", [key])?;
        tx.execute(
            "INSERT INTO records(item_key,payload) VALUES(?1,?2)",
            params![key, serde_json::to_string(&checkpoint)?],
        )?;
    } else if matches!(event.event, ThreadItemEventKind::ItemStarted { .. }) {
        signature::refresh_visibility(tx, &event.turn_id, &signature, &signature)?;
    }
    Ok(())
}

fn read_sync(
    dir: &Path,
    thread: &str,
    options: ThreadHistoryReadOptions,
) -> anyhow::Result<Option<ThreadHistoryPage>> {
    validate_thread_id(thread)?;
    let metadata = match std::fs::read(dir.join("metadata.json")) {
        Ok(bytes) => serde_json::from_slice::<ThreadMetadata>(&bytes)?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    ensure!(
        metadata.thread_id == thread,
        "history metadata thread id mismatch"
    );
    let mut db = open(dir)?;
    synchronize(&mut db, dir)?;
    let tx = db.transaction()?;
    let anchor = if let Some(before) = options.before.as_deref() {
        let cursor = decode(before)?;
        ensure!(
            cursor.thread == thread,
            "history cursor belongs to a different thread"
        );
        Some(
            tx.query_row(
                "SELECT i.turn_ns,i.turn_id,i.ns,i.source,i.key FROM items i \
                WHERE i.turn_id=?1 AND i.source=?2 AND i.item_id=?3",
                params![cursor.turn, cursor.source, cursor.item],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, i64>(4)?,
                    ))
                },
            )
            .optional()?
            .context("history cursor no longer exists")?,
        )
    } else {
        None
    };
    let boundary = anchor.unwrap_or((i64::MAX, String::new(), i64::MAX, i64::MAX, i64::MAX));
    let sql = format!(
        "SELECT i.key,t.payload,i.source,i.item_id,i.bytes FROM items i \
        JOIN turns t ON t.id=i.turn_id WHERE {VISIBLE} \
        AND (i.turn_ns,i.turn_id,i.ns,i.source,i.key)<(?1,?2,?3,?4,?5) \
        ORDER BY i.turn_ns DESC,i.turn_id DESC,i.ns DESC,i.source DESC,i.key DESC LIMIT ?6"
    );
    let mut statement = tx.prepare(&sql)?;
    let limit = options.limit.clamp(1, 200);
    let mut rows = statement.query(params![
        boundary.0,
        boundary.1,
        boundary.2,
        boundary.3,
        boundary.4,
        (limit + 1) as i64
    ])?;
    let mut selected: Vec<(ThreadHistoryTurn, ThreadItem, Cursor)> = Vec::new();
    let mut bytes = 0;
    let mut has_more = false;
    while let Some(row) = rows.next()? {
        if selected.len() == limit {
            has_more = true;
            break;
        }
        let estimated = usize::try_from(row.get::<_, i64>(4)?)?;
        if !selected.is_empty() && bytes + estimated > options.max_bytes.clamp(1, 2 * 1024 * 1024) {
            has_more = true;
            break;
        }
        let mut item = project_item(&tx, row.get(0)?)?;
        signature::set_id(&mut item, row.get(3)?);
        let size = serde_json::to_vec(&item)?.len();
        if !selected.is_empty() && bytes + size > options.max_bytes.clamp(1, 2 * 1024 * 1024) {
            has_more = true;
            break;
        }
        bytes += size;
        let turn: ThreadHistoryTurn = serde_json::from_str(&row.get::<_, String>(1)?)?;
        let cursor = Cursor {
            version: 1,
            thread: thread.into(),
            turn: turn.turn_id.clone(),
            source: row.get(2)?,
            item: row.get(3)?,
        };
        selected.push((turn, item, cursor));
    }
    let before = if has_more {
        selected.last().map(|(_, _, c)| encode(c)).transpose()?
    } else {
        None
    };
    let mut turns: Vec<ThreadHistoryTurn> = Vec::new();
    for (mut turn, item, _) in selected.into_iter().rev() {
        if let Some(previous) = turns.last_mut().filter(|t| t.turn_id == turn.turn_id) {
            previous.items.push(item);
        } else {
            turn.items.push(item);
            turns.push(turn);
        }
    }
    if options.before.is_none() {
        // Empty live turns still carry status even before their first item.
        let latest = tx
            .query_row(
                "SELECT payload FROM turns ORDER BY ns DESC,id DESC LIMIT 1",
                [],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        if let Some(latest) = latest {
            let latest: ThreadHistoryTurn = serde_json::from_str(&latest)?;
            if !turns.iter().any(|t| t.turn_id == latest.turn_id) {
                turns.push(latest);
            }
        }
    }
    Ok(Some(ThreadHistoryPage {
        metadata,
        turns,
        before,
        has_more,
    }))
}
