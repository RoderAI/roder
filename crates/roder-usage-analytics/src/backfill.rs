//! Idempotent analytics backfill from per-thread `events.jsonl` files
//! (roadmap phase 73, Task 3).
//!
//! Raw thread-event JSONL stays the audit source of truth; this module
//! replays it into the SQLite analytics store. Import offsets make repeated
//! backfills incremental, `rebuild` clears analytics rows and replays from
//! scratch, and `best_effort` reports corrupt lines with file/line evidence
//! instead of failing the whole import.

use std::path::{Path, PathBuf};

use anyhow::Context;
use roder_api::events::EventEnvelope;
use roder_api::thread::ThreadMetadata;

use crate::ingest::AnalyticsIngestor;
use crate::model::SessionRecord;
use crate::store::AnalyticsStore;

#[derive(Debug, Clone, Copy, Default)]
pub struct BackfillOptions {
    /// Clear all analytics rows before replaying JSONL.
    pub rebuild: bool,
    /// Report corrupt/unknown lines and continue instead of failing.
    pub best_effort: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BackfillReport {
    pub files_scanned: u64,
    pub lines_ingested: u64,
    pub lines_skipped_by_offset: u64,
    pub sessions_enriched: u64,
    pub parse_errors: Vec<BackfillParseError>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackfillParseError {
    pub path: String,
    pub line: u64,
    pub message: String,
}

/// Replays every `events.jsonl` under `thread_root` (including archived
/// threads) into `store`, then enriches session rows from each thread's
/// `metadata.json` (workspace label, provider, model).
pub fn backfill_analytics(
    thread_root: &Path,
    store: &AnalyticsStore,
    options: BackfillOptions,
) -> anyhow::Result<BackfillReport> {
    if options.rebuild {
        store.clear_all()?;
    }
    let mut report = BackfillReport::default();
    let ingestor = AnalyticsIngestor::new(store);

    for events_path in find_event_logs(thread_root)? {
        report.files_scanned += 1;
        let source_key = events_path.display().to_string();
        let already = store.import_offset(&source_key)?.unwrap_or(0);
        let contents = std::fs::read_to_string(&events_path)
            .with_context(|| format!("read {}", events_path.display()))?;

        let mut line_number = 0_u64;
        for line in contents.lines() {
            line_number += 1;
            if line_number <= already {
                report.lines_skipped_by_offset += 1;
                continue;
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            match serde_json::from_str::<EventEnvelope>(trimmed) {
                Ok(envelope) => {
                    ingestor.ingest_event(&envelope)?;
                    report.lines_ingested += 1;
                }
                Err(error) => {
                    let parse_error = BackfillParseError {
                        path: source_key.clone(),
                        line: line_number,
                        message: error.to_string(),
                    };
                    if options.best_effort {
                        report.parse_errors.push(parse_error);
                    } else {
                        anyhow::bail!(
                            "corrupt event at {}:{}: {} (rerun with --best-effort to skip)",
                            parse_error.path,
                            parse_error.line,
                            parse_error.message
                        );
                    }
                }
            }
        }
        let mtime_ms = std::fs::metadata(&events_path)
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| {
                modified
                    .duration_since(std::time::UNIX_EPOCH)
                    .ok()
                    .map(|duration| duration.as_millis() as i64)
            });
        store.record_import_offset(&source_key, line_number, mtime_ms)?;

        // Enrich the session row from thread metadata when present.
        if let Some(metadata) = read_thread_metadata(&events_path) {
            let (workspace_key, workspace_label) =
                store.workspace_label_mode.label(&metadata.workspace);
            store.upsert_session(&SessionRecord {
                thread_id: metadata.thread_id.clone(),
                workspace_key: Some(workspace_key),
                workspace_label: Some(workspace_label),
                provider: metadata.provider.clone(),
                model: metadata.model.clone(),
                created_at_ms: (metadata.created_at.unix_timestamp_nanos() / 1_000_000) as i64,
                updated_at_ms: (metadata.updated_at.unix_timestamp_nanos() / 1_000_000) as i64,
            })?;
            report.sessions_enriched += 1;
        }
    }
    Ok(report)
}

fn read_thread_metadata(events_path: &Path) -> Option<ThreadMetadata> {
    let metadata_path = events_path.parent()?.join("metadata.json");
    let data = std::fs::read(metadata_path).ok()?;
    serde_json::from_slice(&data).ok()
}

/// Finds every `events.jsonl` under `root` (bounded depth: thread dirs and
/// the archived-threads subtree).
fn find_event_logs(root: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut logs = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0_u8)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if depth < 3 {
                    stack.push((path, depth + 1));
                }
            } else if path.file_name().is_some_and(|name| name == "events.jsonl") {
                logs.push(path);
            }
        }
    }
    logs.sort();
    Ok(logs)
}
