//! `GbrainStore` — the bi-temporal, hybrid-retrieval memory store.
//!
//! Implements the generic [`MemoryStore`] trait (so it slots into roder's memory
//! plumbing additively) plus an inherent bi-temporal API (`capture`, `recall`,
//! `as_of`, `supersede`, `history`, `contradictions`, `consolidate`) used by the
//! `gbrain_*` tools and the `roder-gbrain` CLI.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Context;
use roder_api::embeddings::EmbeddingProvider;
use roder_api::extension::MemoryStoreId;
use roder_api::memory::{
    MemoryCitation, MemoryId, MemoryQuery, MemoryRecord, MemoryScope, MemorySearchResult,
    MemoryStore, MemoryStoreFactory,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::OffsetDateTime;

use crate::dream::{DreamMode, DreamPolicy, DreamStatus};
use crate::embed::{Embedder, Embedding};
use crate::import::{
    DedupeMode, ImportBatchInput, ImportBatchParams, ImportBatchResult, JsonlImportRecord,
};
use crate::model::{AsOf, FactStatus, TemporalFact, content_hash, format_time, parse_time};
use crate::retrieval::{Candidate, Scored, fuse};
use crate::schema;

const SUPERSEDES: &str = "supersedes";
const CONTRADICTS: &str = "contradicts";

/// Event-cluster expansion: how many top hits seed the thread set, and the cap
/// on sibling facts pulled in. A justification/provenance question's evidence is
/// the cluster of artifacts sharing a thread/event, so we surface that cluster.
const EXPANSION_SEED_HITS: usize = 3;
const EXPANSION_CAP: usize = 12;

/// Parameters for capturing a new fact.
#[derive(Debug, Clone)]
pub struct CaptureInput {
    pub scope: MemoryScope,
    pub subject: Option<String>,
    pub text: String,
    pub metadata: Value,
    pub valid_at: Option<OffsetDateTime>,
    pub invalid_at: Option<OffsetDateTime>,
    pub ingested_at: Option<OffsetDateTime>,
    pub provenance: Vec<String>,
    pub supersedes: Option<String>,
    pub supersession_reason: Option<String>,
}

impl CaptureInput {
    pub fn new(scope: MemoryScope, text: impl Into<String>) -> Self {
        Self {
            scope,
            subject: None,
            text: text.into(),
            metadata: Value::Null,
            valid_at: None,
            invalid_at: None,
            ingested_at: None,
            provenance: Vec::new(),
            supersedes: None,
            supersession_reason: None,
        }
    }
}

/// Parameters for a recall / as-of query.
#[derive(Debug, Clone)]
pub struct RecallParams {
    pub query: String,
    pub as_of: AsOf,
    pub scope: Option<MemoryScope>,
    pub include_global: bool,
    pub limit: usize,
    /// Pull in the top hits' event/thread cluster (the full evidence chain).
    /// Helps evidence-enumeration questions (C5/C2/C4) but dilutes focused-fact
    /// questions (C1/C3), so it's opt-in — the caller decides per question.
    pub expand: bool,
}

/// A detected contradiction between two coexisting facts about the same subject.
pub struct ContradictionPair {
    pub a: TemporalFact,
    pub b: TemporalFact,
}

/// Result of a recall / as-of query.
pub struct RecallResult {
    pub hits: Vec<Scored>,
    pub contradictions: Vec<ContradictionPair>,
    pub as_of: AsOf,
    pub now: OffsetDateTime,
}

/// Counts from a consolidation pass.
#[derive(Debug, Default, Clone, Copy)]
pub struct ConsolidateStats {
    pub supersession_links: usize,
    pub contradiction_links: usize,
}

#[derive(Debug, Clone)]
pub struct DreamParams {
    pub mode: DreamMode,
    pub scope: MemoryScope,
    pub since: Option<OffsetDateTime>,
    pub run_policy: DreamPolicy,
    pub workers: usize,
    pub dry_run: bool,
    pub cancellation_token: Option<String>,
    pub reasoner_model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DreamRunReport {
    pub id: String,
    pub scope_id: String,
    pub mode: DreamMode,
    pub status: DreamStatus,
    pub algorithm_version: String,
    pub run_policy: DreamPolicy,
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub finished_at: Option<OffsetDateTime>,
    pub workers: usize,
    pub input_fact_count: usize,
    pub derived_statement_count: usize,
    pub derived_event_count: usize,
    pub invalidated_event_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoner_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// --------------------------------------------------------------------------- //
// Factory
// --------------------------------------------------------------------------- //

pub struct GbrainStoreFactory {
    base_path: PathBuf,
    provider: Option<Arc<dyn EmbeddingProvider>>,
}

impl GbrainStoreFactory {
    pub fn new(base_path: PathBuf, provider: Option<Arc<dyn EmbeddingProvider>>) -> Self {
        Self {
            base_path,
            provider,
        }
    }
}

impl MemoryStoreFactory for GbrainStoreFactory {
    fn id(&self) -> MemoryStoreId {
        "gbrain-bitemporal".to_string()
    }

    fn create(&self) -> Arc<dyn MemoryStore> {
        Arc::new(
            GbrainStore::open(
                self.base_path.join("gbrain.sqlite3"),
                Embedder::new(self.provider.clone()),
            )
            .expect("open gbrain store"),
        )
    }
}

// --------------------------------------------------------------------------- //
// Store
// --------------------------------------------------------------------------- //

pub struct GbrainStore {
    path: PathBuf,
    conn: Mutex<Connection>,
    embedder: Embedder,
}

impl GbrainStore {
    pub fn open(path: PathBuf, embedder: Embedder) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(&path).with_context(|| format!("open {}", path.display()))?;
        schema::migrate(&conn)?;
        Ok(Self {
            path,
            conn: Mutex::new(conn),
            embedder,
        })
    }

    /// In-memory store (tests / ephemeral CLI use).
    pub fn open_in_memory(embedder: Embedder) -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory()?;
        schema::migrate(&conn)?;
        Ok(Self {
            path: PathBuf::from(":memory:"),
            conn: Mutex::new(conn),
            embedder,
        })
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    pub(crate) fn with_conn<T>(
        &self,
        f: impl FnOnce(&Connection) -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("gbrain sqlite connection lock poisoned"))?;
        f(&conn)
    }

    // ---------------------------------------------------------------------- //
    // Bi-temporal API
    // ---------------------------------------------------------------------- //

    /// Capture a new fact (embedding computed before the DB lock is taken).
    pub async fn capture(&self, input: CaptureInput) -> anyhow::Result<TemporalFact> {
        let embedding = self.embedder.embed(&input.text).await;
        self.with_conn(|conn| capture_blocking(conn, input, embedding))
    }

    /// Hybrid recall over the snapshot defined by `params.as_of`.
    pub async fn recall(&self, params: RecallParams) -> anyhow::Result<RecallResult> {
        let query_embedding = self.embedder.embed(&params.query).await;
        self.with_conn(|conn| recall_blocking(conn, &params, &query_embedding))
    }

    /// Reconstruct the belief snapshot as of `instant`.
    pub async fn as_of(
        &self,
        instant: OffsetDateTime,
        query: &str,
        scope: Option<MemoryScope>,
        limit: usize,
    ) -> anyhow::Result<RecallResult> {
        self.recall(RecallParams {
            query: query.to_string(),
            as_of: AsOf::at(instant),
            scope,
            include_global: true,
            limit,
            expand: false,
        })
        .await
    }

    /// Replace `old_id` with a new fact, recording the supersession link + reason.
    pub async fn supersede(
        &self,
        old_id: &str,
        new_text: impl Into<String>,
        reason: impl Into<String>,
        new_valid_at: Option<OffsetDateTime>,
    ) -> anyhow::Result<TemporalFact> {
        let old = self
            .get_fact(old_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("fact not found: {old_id}"))?;
        let mut input = CaptureInput::new(old.scope.clone(), new_text);
        input.subject = old.subject.clone();
        input.valid_at = new_valid_at;
        // The correction is RECORDED now, regardless of when the new fact became
        // true (`valid_at` may be backdated). Pin transaction time to now so a
        // transaction-time as-of query doesn't see the correction before it existed.
        input.ingested_at = Some(OffsetDateTime::now_utc());
        input.provenance = old.provenance.clone();
        input.supersedes = Some(old_id.to_string());
        input.supersession_reason = Some(reason.into());
        self.capture(input).await
    }

    /// Load a single fact (any status) by id.
    pub async fn get_fact(&self, id: &str) -> anyhow::Result<Option<TemporalFact>> {
        self.with_conn(|conn| load_fact(conn, id))
    }

    /// Full timeline for a subject (or the supersession chain through `id`),
    /// including invalidated / retracted versions, oldest first.
    pub async fn history(
        &self,
        id: Option<&str>,
        subject: Option<&str>,
        scope: Option<MemoryScope>,
    ) -> anyhow::Result<Vec<TemporalFact>> {
        self.with_conn(|conn| {
            let subject = match (subject, id) {
                (Some(s), _) => Some(s.to_string()),
                (None, Some(id)) => load_fact(conn, id)?.and_then(|f| f.subject),
                (None, None) => None,
            };
            let mut facts = load_facts(conn, scope.as_ref(), false)?;
            if let Some(subject) = subject {
                facts.retain(|f| f.subject.as_deref() == Some(subject.as_str()));
            } else if let Some(id) = id {
                // No subject: walk the supersession chain through this id.
                let chain = supersession_chain(conn, id)?;
                facts.retain(|f| chain.contains(&f.id));
            }
            facts.sort_by(|a, b| a.valid_at.cmp(&b.valid_at).then(a.id.cmp(&b.id)));
            Ok(facts)
        })
    }

    /// Currently-believed contradictions (computed live, independent of
    /// `consolidate`).
    pub async fn contradictions(
        &self,
        scope: Option<MemoryScope>,
        subject: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<Vec<ContradictionPair>> {
        self.with_conn(|conn| {
            let now = OffsetDateTime::now_utc();
            let mut facts = load_facts(conn, scope.as_ref(), false)?;
            if let Some(subject) = subject {
                facts.retain(|f| f.subject.as_deref() == Some(subject));
            }
            let pairs = detect_contradictions(&facts, now);
            Ok(pairs
                .into_iter()
                .take(limit.max(1))
                .map(|(i, j)| ContradictionPair {
                    a: facts[i].clone(),
                    b: facts[j].clone(),
                })
                .collect())
        })
    }

    /// The gbrain `extract`/`dream` analog: deterministically (re)build the
    /// supersession + contradiction link graph. Idempotent.
    pub async fn consolidate(
        &self,
        scope: Option<MemoryScope>,
    ) -> anyhow::Result<ConsolidateStats> {
        self.with_conn(|conn| {
            let now = OffsetDateTime::now_utc();
            let facts = load_facts(conn, scope.as_ref(), false)?;
            let mut stats = ConsolidateStats::default();

            // 1. Supersession links from each fact's `supersedes` pointer.
            for fact in &facts {
                if let Some(old) = &fact.supersedes {
                    let inserted = conn.execute(
                        "INSERT OR IGNORE INTO gbrain_links(from_id, to_id, kind, reason, created_at)
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![
                            fact.id,
                            old,
                            SUPERSEDES,
                            fact.supersession_reason,
                            format_time(now)
                        ],
                    )?;
                    stats.supersession_links += inserted;
                }
            }

            // 2. Contradiction links among coexisting same-subject facts.
            for (i, j) in detect_contradictions(&facts, now) {
                let (from, to) = canonical_pair(&facts[i].id, &facts[j].id);
                let inserted = conn.execute(
                    "INSERT OR IGNORE INTO gbrain_links(from_id, to_id, kind, reason, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![from, to, CONTRADICTS, Option::<String>::None, format_time(now)],
                )?;
                stats.contradiction_links += inserted;
            }
            Ok(stats)
        })
    }

    /// Import raw facts from a batch payload. JSONL file/string input is
    /// supported now; directory import is reserved for the graph-shaped importer.
    pub async fn import_batch(
        &self,
        params: ImportBatchParams,
    ) -> anyhow::Result<ImportBatchResult> {
        if params.format != "jsonl" {
            anyhow::bail!(
                "unsupported import format {:?}; jsonl is implemented, directory is TODO",
                params.format
            );
        }
        let source_path = match &params.input {
            ImportBatchInput::Path(path) => {
                if path.is_dir() {
                    anyhow::bail!("directory import is TODO for phase 84");
                }
                Some(path.display().to_string())
            }
            ImportBatchInput::JsonlString(_) => None,
        };
        let body = match &params.input {
            ImportBatchInput::Path(path) => std::fs::read_to_string(path)
                .with_context(|| format!("read import input {}", path.display()))?,
            ImportBatchInput::JsonlString(payload) => payload.clone(),
        };
        let source_hash = content_hash(&body);
        let run_id = uuid::Uuid::new_v4().to_string();
        let started_at = OffsetDateTime::now_utc();
        self.with_conn(|conn| {
            let scope_id = ensure_scope(conn, &params.scope)?;
            conn.execute(
                "INSERT INTO gbrain_import_runs(id, scope_id, source_path, source_hash, started_at, status, metadata)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'running', ?6)",
                params![
                    run_id,
                    scope_id,
                    source_path,
                    source_hash,
                    format_time(started_at),
                    serde_json::to_string(&json!({
                        "source": params.source,
                        "format": params.format,
                        "dedupe": params.dedupe.as_str(),
                        "dream_after_import": params.dream_after_import,
                        "metadata": params.metadata,
                    }))?,
                ],
            )?;
            Ok(())
        })?;

        let mut total = 0usize;
        let mut inserted = 0usize;
        let mut skipped_duplicates = 0usize;
        let mut errors = 0usize;
        let mut fact_ids = Vec::new();

        for (line_index, line) in body.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            total += 1;
            let record: JsonlImportRecord = match serde_json::from_str(line) {
                Ok(record) => record,
                Err(_) => {
                    errors += 1;
                    continue;
                }
            };
            let record_hash = content_hash(&record.text);
            let source_id = record.source_id.clone();
            let duplicate = self.with_conn(|conn| {
                import_duplicate_exists(
                    conn,
                    &params.scope,
                    params.dedupe,
                    source_id.as_deref(),
                    &record_hash,
                )
            })?;
            if duplicate {
                skipped_duplicates += 1;
                continue;
            }

            let mut metadata = match record.metadata {
                Value::Object(_) => record.metadata,
                _ => json!({}),
            };
            if let Value::Object(map) = &mut metadata {
                if let Some(source_id) = &record.source_id {
                    map.insert("source_id".into(), json!(source_id));
                }
                if let Some(source) = &params.source {
                    map.insert("source".into(), json!(source));
                }
                if let Some(thread_id) = &record.thread_id {
                    map.insert("thread_id".into(), json!(thread_id));
                }
                if let Some(slug) = &record.slug {
                    map.insert("slug".into(), json!(slug));
                }
                map.insert("import_run_id".into(), json!(run_id));
                map.insert("import_line".into(), json!(line_index + 1));
            }

            let mut provenance = record.provenance;
            if let Some(slug) = &record.slug
                && !provenance.iter().any(|item| item == slug)
            {
                provenance.insert(0, slug.clone());
            }
            if let Some(source_id) = &record.source_id {
                let marker = format!("source_id:{source_id}");
                if !provenance.iter().any(|item| item == &marker) {
                    provenance.push(marker);
                }
            }

            let mut input = CaptureInput::new(params.scope.clone(), record.text);
            input.subject = record.subject;
            input.metadata = metadata;
            input.provenance = provenance;
            input.valid_at = record
                .valid_at
                .as_deref()
                .or(record.timestamp.as_deref())
                .map(crate::model::parse_flexible)
                .transpose()?;
            input.invalid_at = record
                .invalid_at
                .as_deref()
                .map(crate::model::parse_flexible)
                .transpose()?;
            input.ingested_at = record
                .ingested_at
                .as_deref()
                .map(crate::model::parse_flexible)
                .transpose()?;
            let fact = self.capture(input).await?;
            fact_ids.push(fact.id);
            inserted += 1;
        }

        let status = if errors == 0 { "completed" } else { "failed" };
        let finished_at = OffsetDateTime::now_utc();
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE gbrain_import_runs
                 SET finished_at = ?1, status = ?2, error = ?3, metadata = ?4
                 WHERE id = ?5",
                params![
                    format_time(finished_at),
                    status,
                    if errors == 0 {
                        None::<String>
                    } else {
                        Some(format!("{errors} JSONL record(s) failed to parse"))
                    },
                    serde_json::to_string(&json!({
                        "source": params.source,
                        "format": params.format,
                        "dedupe": params.dedupe.as_str(),
                        "total": total,
                        "inserted": inserted,
                        "skipped_duplicates": skipped_duplicates,
                        "errors": errors,
                        "dream_after_import": params.dream_after_import,
                        "metadata": params.metadata,
                    }))?,
                    run_id,
                ],
            )?;
            Ok(())
        })?;

        Ok(ImportBatchResult {
            run_id,
            status: status.to_string(),
            scope_id: params.scope.stable_id(),
            source: params.source,
            format: params.format,
            dedupe: params.dedupe,
            total,
            inserted,
            skipped_duplicates,
            errors,
            fact_ids,
        })
    }

    /// Run explicit at-rest dream maintenance. This minimal phase-84 runner only
    /// records the ledger and reuses `consolidate` for refine/full link rebuilds.
    pub async fn dream(&self, params: DreamParams) -> anyhow::Result<DreamRunReport> {
        let run_id = uuid::Uuid::new_v4().to_string();
        let started_at = OffsetDateTime::now_utc();
        let workers = params.workers.max(1);
        let input_fact_count = self.with_conn(|conn| {
            ensure_scope(conn, &params.scope)?;
            count_facts_since(conn, &params.scope, params.since)
        })?;
        let scope_id = params.scope.stable_id();
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO gbrain_dream_runs(id, scope_id, mode, started_at, status, algorithm_version,
                    reasoner_model, run_policy, external_cancellation_token, workers, input_fact_count)
                 VALUES (?1, ?2, ?3, ?4, 'running', ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    run_id,
                    scope_id,
                    params.mode.as_str(),
                    format_time(started_at),
                    "phase84-minimal-v1",
                    params.reasoner_model,
                    params.run_policy.as_str(),
                    params.cancellation_token,
                    workers as i64,
                    input_fact_count as i64,
                ],
            )?;
            Ok(())
        })?;

        let mut derived_event_count = 0usize;
        let mut invalidated_event_count = 0usize;
        if !params.dry_run && matches!(params.mode, DreamMode::Refine | DreamMode::Full) {
            let stats = self.consolidate(Some(params.scope.clone())).await?;
            derived_event_count = stats.supersession_links + stats.contradiction_links;
            invalidated_event_count = stats.contradiction_links;
        }

        let finished_at = OffsetDateTime::now_utc();
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE gbrain_dream_runs
                 SET finished_at = ?1, status = 'completed', derived_event_count = ?2, invalidated_event_count = ?3
                 WHERE id = ?4",
                params![
                    format_time(finished_at),
                    derived_event_count as i64,
                    invalidated_event_count as i64,
                    run_id,
                ],
            )?;
            Ok(())
        })?;

        Ok(DreamRunReport {
            id: run_id,
            scope_id,
            mode: params.mode,
            status: DreamStatus::Completed,
            algorithm_version: "phase84-minimal-v1".into(),
            run_policy: params.run_policy,
            started_at,
            finished_at: Some(finished_at),
            workers,
            input_fact_count,
            derived_statement_count: 0,
            derived_event_count,
            invalidated_event_count,
            reasoner_model: params.reasoner_model,
            error: None,
        })
    }

    pub async fn dream_status(&self, run_id: &str) -> anyhow::Result<Option<DreamRunReport>> {
        self.with_conn(|conn| load_dream_run(conn, run_id))
    }

    /// In-place update of a fact's text/metadata (re-embeds). Used by the
    /// generic `MemoryStore::put` update path.
    async fn update_in_place(&self, id: &str, text: String, metadata: Value) -> anyhow::Result<()> {
        let embedding = self.embedder.embed(&text).await;
        self.with_conn(|conn| {
            let now = OffsetDateTime::now_utc();
            let tx = conn.unchecked_transaction()?;
            tx.execute(
                "UPDATE gbrain_facts SET text = ?1, content_hash = ?2, metadata = ?3, updated_at = ?4
                 WHERE id = ?5",
                params![
                    text,
                    content_hash(&text),
                    serde_json::to_string(&metadata)?,
                    format_time(now),
                    id
                ],
            )?;
            upsert_embedding(&tx, id, &embedding, now)?;
            tx.commit()?;
            Ok(())
        })
    }
}

// --------------------------------------------------------------------------- //
// MemoryStore impl (additive integration)
// --------------------------------------------------------------------------- //

#[async_trait::async_trait]
impl MemoryStore for GbrainStore {
    fn id(&self) -> MemoryStoreId {
        "gbrain-bitemporal".to_string()
    }

    async fn put(&self, record: MemoryRecord) -> anyhow::Result<MemoryId> {
        // In-place update when an existing id is supplied.
        if let Some(id) = record.id.clone()
            && self.get_fact(&id).await?.is_some()
        {
            self.update_in_place(&id, record.text, record.metadata)
                .await?;
            return Ok(id);
        }
        let input = capture_input_from_record(record);
        let fact = self.capture(input).await?;
        Ok(fact.id)
    }

    async fn get(&self, id: &MemoryId) -> anyhow::Result<Option<MemoryRecord>> {
        Ok(self.get_fact(id).await?.map(|f| fact_to_record(&f)))
    }

    async fn search(&self, query: MemoryQuery) -> anyhow::Result<Vec<MemorySearchResult>> {
        let result = self
            .recall(RecallParams {
                query: query.text,
                as_of: AsOf::now(),
                scope: query.scope,
                include_global: query.include_global,
                limit: query.limit,
                expand: false,
            })
            .await?;
        Ok(result
            .hits
            .into_iter()
            .map(|scored| {
                let record = fact_to_record(&scored.fact);
                let citation = record.id.clone().map(|memory_id| MemoryCitation {
                    memory_id,
                    scope_id: record.scope.stable_id(),
                    snippet: snippet(&record.text),
                    score_millis: (scored.score.max(0.0) * 1000.0) as u32,
                });
                MemorySearchResult {
                    record,
                    score: scored.score,
                    citation,
                }
            })
            .collect())
    }

    async fn delete(&self, id: &MemoryId) -> anyhow::Result<()> {
        // Invalidated, never deleted: retract the record (transaction time ends).
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE gbrain_facts SET expired_at = ?1, updated_at = ?1 WHERE id = ?2 AND expired_at IS NULL",
                params![format_time(OffsetDateTime::now_utc()), id],
            )?;
            Ok(())
        })
    }

    async fn list(
        &self,
        scope: Option<MemoryScope>,
        limit: usize,
    ) -> anyhow::Result<Vec<MemoryRecord>> {
        let result = self
            .recall(RecallParams {
                query: String::new(),
                as_of: AsOf::now(),
                scope,
                include_global: false,
                limit,
                expand: false,
            })
            .await?;
        Ok(result
            .hits
            .into_iter()
            .map(|scored| fact_to_record(&scored.fact))
            .collect())
    }
}

// --------------------------------------------------------------------------- //
// Blocking helpers (run under the connection lock)
// --------------------------------------------------------------------------- //

fn capture_blocking(
    conn: &Connection,
    input: CaptureInput,
    embedding: Embedding,
) -> anyhow::Result<TemporalFact> {
    let now = OffsetDateTime::now_utc();
    let id = uuid::Uuid::new_v4().to_string();
    let valid_at = input.valid_at.unwrap_or(now);
    // Default the transaction time when not given. A *correction* (supersedes set)
    // is recorded NOW regardless of a backdated valid_at; a plain capture is
    // assumed on-record from the moment it became true (so as-of date-travel works
    // from `valid_at` alone).
    let ingested_at = input.ingested_at.unwrap_or(if input.supersedes.is_some() {
        now
    } else {
        valid_at
    });
    let fact = TemporalFact {
        id: id.clone(),
        scope: input.scope.clone(),
        subject: input.subject,
        text: input.text,
        metadata: if input.metadata.is_null() {
            json!({})
        } else {
            input.metadata
        },
        valid_at,
        invalid_at: input.invalid_at,
        ingested_at,
        expired_at: None,
        supersedes: input.supersedes.clone(),
        superseded_by: None,
        supersession_reason: input.supersession_reason.clone(),
        provenance: input.provenance,
        content_hash: String::new(),
        created_at: now,
        updated_at: now,
    };
    let fact = TemporalFact {
        content_hash: content_hash(&fact.text),
        ..fact
    };

    // All writes for one capture (fact + embedding + supersession update + link)
    // commit atomically, so a partial failure can never leave the bi-temporal
    // invariant (new.supersedes <-> old.superseded_by <-> link) half-applied.
    let tx = conn.unchecked_transaction()?;

    // Validate the supersession target up front: load the predecessor (clean
    // error instead of an orphan replacement fact + FK violation), and clamp its
    // invalid_at so a backdated `valid_at` can't invert the predecessor's interval.
    let predecessor_invalid_at = if let Some(old_id) = &fact.supersedes {
        let old = load_fact(&tx, old_id)?
            .ok_or_else(|| anyhow::anyhow!("supersede target not found: {old_id}"))?;
        Some(fact.valid_at.max(old.valid_at))
    } else {
        None
    };

    let scope_id = ensure_scope(&tx, &fact.scope)?;
    tx.execute(
        "INSERT INTO gbrain_facts(id, scope_id, subject, text, content_hash, metadata,
            valid_at, invalid_at, ingested_at, expired_at, supersedes, superseded_by,
            supersession_reason, provenance, created_at, updated_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
        params![
            fact.id,
            scope_id,
            fact.subject,
            fact.text,
            fact.content_hash,
            serde_json::to_string(&fact.metadata)?,
            format_time(fact.valid_at),
            fact.invalid_at.map(format_time),
            format_time(fact.ingested_at),
            fact.expired_at.map(format_time),
            fact.supersedes,
            fact.superseded_by,
            fact.supersession_reason,
            serde_json::to_string(&fact.provenance)?,
            format_time(fact.created_at),
            format_time(fact.updated_at),
        ],
    )?;
    upsert_embedding(&tx, &fact.id, &embedding, now)?;

    // Wire up supersession: invalidate + link the predecessor.
    if let Some(old_id) = &fact.supersedes {
        let invalid_at = predecessor_invalid_at.expect("predecessor loaded above");
        tx.execute(
            "UPDATE gbrain_facts
             SET superseded_by = ?1,
                 invalid_at = COALESCE(invalid_at, ?2),
                 updated_at = ?3
             WHERE id = ?4",
            params![fact.id, format_time(invalid_at), format_time(now), old_id],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO gbrain_links(from_id, to_id, kind, reason, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                fact.id,
                old_id,
                SUPERSEDES,
                fact.supersession_reason,
                format_time(now)
            ],
        )?;
    }
    tx.commit()?;
    Ok(fact)
}

fn recall_blocking(
    conn: &Connection,
    params: &RecallParams,
    query_embedding: &Embedding,
) -> anyhow::Result<RecallResult> {
    let now = OffsetDateTime::now_utc();
    let mut facts = load_facts(conn, params.scope.as_ref(), params.include_global)?;
    facts.retain(|fact| params.as_of.visible(fact, now));

    let limit = params.limit.max(1);
    let mut hits: Vec<Scored> = if params.query.trim().is_empty() {
        // List mode: most-recently-valid first.
        let mut listed: Vec<Scored> = facts
            .iter()
            .cloned()
            .map(|fact| Scored {
                fact,
                score: 1.0,
                vector_score: 0.0,
                lexical_score: 0.0,
            })
            .collect();
        listed.sort_by(|a, b| {
            b.fact
                .valid_at
                .cmp(&a.fact.valid_at)
                .then(a.fact.id.cmp(&b.fact.id))
        });
        listed.truncate(limit);
        listed
    } else {
        let neighbors = load_neighbors(conn)?;
        let candidates: Vec<Candidate> = facts
            .iter()
            .map(|fact| {
                let vector = load_embedding(
                    conn,
                    &fact.id,
                    &query_embedding.provider_id,
                    &query_embedding.model,
                )
                .ok()
                .flatten();
                Candidate {
                    fact: fact.clone(),
                    vector,
                }
            })
            .collect();
        let mut scored = fuse(
            &params.query,
            &query_embedding.values,
            candidates,
            &neighbors,
        );
        scored.retain(|s| s.score > 0.0);
        scored.truncate(limit);
        scored
    };

    // Event-cluster expansion: pull in sibling facts sharing the top hits'
    // thread/event so the full evidence chain is surfaced (the main C5/C2
    // retrieval-miss), bounded by EXPANSION_CAP to keep context tight.
    if params.expand && !params.query.trim().is_empty() {
        let thread_of = |f: &TemporalFact| {
            f.metadata
                .get("thread_id")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        };
        let top_threads: HashSet<String> = hits
            .iter()
            .take(EXPANSION_SEED_HITS)
            .filter_map(|s| thread_of(&s.fact))
            .collect();
        if !top_threads.is_empty() {
            let present: HashSet<String> = hits.iter().map(|s| s.fact.id.clone()).collect();
            let mut siblings: Vec<TemporalFact> = facts
                .iter()
                .filter(|f| !present.contains(&f.id))
                .filter(|f| thread_of(f).is_some_and(|t| top_threads.contains(&t)))
                .cloned()
                .collect();
            // Chronological so the cluster reads as the event unfolded.
            siblings.sort_by(|a, b| a.valid_at.cmp(&b.valid_at).then(a.id.cmp(&b.id)));
            siblings.truncate(EXPANSION_CAP);
            for fact in siblings {
                hits.push(Scored {
                    fact,
                    score: 0.0,
                    vector_score: 0.0,
                    lexical_score: 0.0,
                });
            }
        }
    }

    // Contradictions relevant to the returned hits.
    let hit_subjects: Vec<String> = hits.iter().filter_map(|s| s.fact.subject.clone()).collect();
    let relevant: Vec<TemporalFact> = facts
        .iter()
        .filter(|f| f.subject.as_ref().is_some_and(|s| hit_subjects.contains(s)))
        .cloned()
        .collect();
    // Detect contradictions at the SAME transaction-time point the hits were
    // filtered at, so a past as-of snapshot reports the conflicts that existed
    // then (not those gated by wall-clock now).
    let anchor_tt = params.as_of.transaction_time.unwrap_or(now);
    let contradictions = detect_contradictions(&relevant, anchor_tt)
        .into_iter()
        .map(|(i, j)| ContradictionPair {
            a: relevant[i].clone(),
            b: relevant[j].clone(),
        })
        .collect();

    Ok(RecallResult {
        hits,
        contradictions,
        as_of: params.as_of,
        now,
    })
}

// --------------------------------------------------------------------------- //
// SQL helpers
// --------------------------------------------------------------------------- //

pub(crate) fn ensure_scope(conn: &Connection, scope: &MemoryScope) -> anyhow::Result<String> {
    let (kind, value, label) = match scope {
        MemoryScope::Global => ("global", None, "Global memory".to_string()),
        MemoryScope::User(v) => ("user", Some(v.as_str()), format!("User memory: {v}")),
        MemoryScope::Workspace(v) => (
            "workspace",
            Some(v.as_str()),
            format!("Workspace memory: {v}"),
        ),
        MemoryScope::Project(v) => ("project", Some(v.as_str()), format!("Project memory: {v}")),
        MemoryScope::Thread(v) => ("thread", Some(v.as_str()), format!("Thread memory: {v}")),
    };
    conn.execute(
        "INSERT OR IGNORE INTO gbrain_scopes(id, kind, value, label, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            scope.stable_id(),
            kind,
            value,
            label,
            format_time(OffsetDateTime::now_utc())
        ],
    )?;
    Ok(scope.stable_id())
}

fn upsert_embedding(
    conn: &Connection,
    fact_id: &str,
    embedding: &Embedding,
    now: OffsetDateTime,
) -> anyhow::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO gbrain_embeddings(fact_id, provider_id, model, dimensions, embedding, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            fact_id,
            embedding.provider_id,
            embedding.model,
            embedding.values.len() as i64,
            crate::embed::encode(&embedding.values),
            format_time(now),
        ],
    )?;
    Ok(())
}

fn load_embedding(
    conn: &Connection,
    fact_id: &str,
    provider_id: &str,
    model: &str,
) -> anyhow::Result<Option<Vec<f32>>> {
    let bytes: Option<Vec<u8>> = conn
        .query_row(
            "SELECT embedding FROM gbrain_embeddings WHERE fact_id = ?1 AND provider_id = ?2 AND model = ?3",
            params![fact_id, provider_id, model],
            |row| row.get(0),
        )
        .optional()?;
    Ok(bytes.map(|b| crate::embed::decode(&b)))
}

const FACT_COLUMNS: &str = "f.id, s.kind, s.value, f.subject, f.text, f.content_hash, f.metadata, \
     f.valid_at, f.invalid_at, f.ingested_at, f.expired_at, f.supersedes, f.superseded_by, \
     f.supersession_reason, f.provenance, f.created_at, f.updated_at";

fn load_fact(conn: &Connection, id: &str) -> anyhow::Result<Option<TemporalFact>> {
    let sql = format!(
        "SELECT {FACT_COLUMNS} FROM gbrain_facts f JOIN gbrain_scopes s ON s.id = f.scope_id WHERE f.id = ?1"
    );
    conn.query_row(&sql, params![id], row_to_fact)
        .optional()
        .map_err(Into::into)
}

fn load_facts(
    conn: &Connection,
    scope: Option<&MemoryScope>,
    include_global: bool,
) -> anyhow::Result<Vec<TemporalFact>> {
    let base = format!(
        "SELECT {FACT_COLUMNS} FROM gbrain_facts f JOIN gbrain_scopes s ON s.id = f.scope_id"
    );
    let mut facts = Vec::new();
    match scope {
        // Push scope filtering into SQL (uses idx_gbrain_facts_scope) instead of
        // loading the whole table and filtering in Rust.
        Some(scope) if include_global && *scope != MemoryScope::Global => {
            let sql = format!("{base} WHERE f.scope_id = ?1 OR f.scope_id = 'global'");
            let mut stmt = conn.prepare(&sql)?;
            for row in stmt.query_map(params![scope.stable_id()], row_to_fact)? {
                facts.push(row?);
            }
        }
        Some(scope) => {
            let sql = format!("{base} WHERE f.scope_id = ?1");
            let mut stmt = conn.prepare(&sql)?;
            for row in stmt.query_map(params![scope.stable_id()], row_to_fact)? {
                facts.push(row?);
            }
        }
        None => {
            let mut stmt = conn.prepare(&base)?;
            for row in stmt.query_map([], row_to_fact)? {
                facts.push(row?);
            }
        }
    }
    Ok(facts)
}

/// Adjacency: fact id -> linked fact ids (both directions, any kind).
fn load_neighbors(conn: &Connection) -> anyhow::Result<HashMap<String, Vec<String>>> {
    let mut stmt = conn.prepare("SELECT from_id, to_id FROM gbrain_links")?;
    let mut neighbors: HashMap<String, Vec<String>> = HashMap::new();
    for row in stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })? {
        let (from, to) = row?;
        neighbors.entry(from.clone()).or_default().push(to.clone());
        neighbors.entry(to).or_default().push(from);
    }
    Ok(neighbors)
}

fn import_duplicate_exists(
    conn: &Connection,
    scope: &MemoryScope,
    dedupe: DedupeMode,
    source_id: Option<&str>,
    hash: &str,
) -> anyhow::Result<bool> {
    let scope_id = scope.stable_id();
    let by_source_id = if let Some(source_id) = source_id {
        conn.query_row(
            "SELECT 1 FROM gbrain_facts
             WHERE scope_id = ?1
               AND json_extract(metadata, '$.source_id') = ?2
             LIMIT 1",
            params![scope_id, source_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some()
    } else {
        false
    };
    let by_content_hash = conn
        .query_row(
            "SELECT 1 FROM gbrain_facts WHERE scope_id = ?1 AND content_hash = ?2 LIMIT 1",
            params![scope_id, hash],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    Ok(match dedupe {
        DedupeMode::SourceId => by_source_id,
        DedupeMode::ContentHash => by_content_hash,
        DedupeMode::Both => by_source_id || by_content_hash,
    })
}

pub(crate) fn count_facts_since(
    conn: &Connection,
    scope: &MemoryScope,
    since: Option<OffsetDateTime>,
) -> anyhow::Result<usize> {
    let count: i64 = if let Some(since) = since {
        conn.query_row(
            "SELECT COUNT(*) FROM gbrain_facts WHERE scope_id = ?1 AND ingested_at >= ?2",
            params![scope.stable_id(), format_time(since)],
            |row| row.get(0),
        )?
    } else {
        conn.query_row(
            "SELECT COUNT(*) FROM gbrain_facts WHERE scope_id = ?1",
            params![scope.stable_id()],
            |row| row.get(0),
        )?
    };
    Ok(count.max(0) as usize)
}

fn load_dream_run(conn: &Connection, run_id: &str) -> anyhow::Result<Option<DreamRunReport>> {
    conn.query_row(
        "SELECT id, scope_id, mode, started_at, finished_at, status, algorithm_version,
                reasoner_model, run_policy, workers, input_fact_count, derived_statement_count,
                derived_event_count, invalidated_event_count, error
         FROM gbrain_dream_runs
         WHERE id = ?1",
        params![run_id],
        |row| {
            let mode: String = row.get(2)?;
            let started_at: String = row.get(3)?;
            let finished_at: Option<String> = row.get(4)?;
            let status: String = row.get(5)?;
            let run_policy: String = row.get(8)?;
            let workers: i64 = row.get(9)?;
            let input_fact_count: i64 = row.get(10)?;
            let derived_statement_count: i64 = row.get(11)?;
            let derived_event_count: i64 = row.get(12)?;
            let invalidated_event_count: i64 = row.get(13)?;
            Ok(DreamRunReport {
                id: row.get(0)?,
                scope_id: row.get(1)?,
                mode: parse_dream_mode(&mode),
                started_at: parse_time(&started_at).unwrap_or(OffsetDateTime::UNIX_EPOCH),
                finished_at: finished_at.and_then(|value| parse_time(&value).ok()),
                status: parse_dream_status(&status),
                algorithm_version: row.get(6)?,
                reasoner_model: row.get(7)?,
                run_policy: parse_dream_policy(&run_policy),
                workers: workers.max(0) as usize,
                input_fact_count: input_fact_count.max(0) as usize,
                derived_statement_count: derived_statement_count.max(0) as usize,
                derived_event_count: derived_event_count.max(0) as usize,
                invalidated_event_count: invalidated_event_count.max(0) as usize,
                error: row.get(14)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

fn parse_dream_mode(value: &str) -> DreamMode {
    match value {
        "refine" => DreamMode::Refine,
        "compact" => DreamMode::Compact,
        "full" => DreamMode::Full,
        _ => DreamMode::Enrich,
    }
}

fn parse_dream_policy(value: &str) -> DreamPolicy {
    match value {
        "eval" => DreamPolicy::Eval,
        "import" => DreamPolicy::Import,
        "maintenance" => DreamPolicy::Maintenance,
        _ => DreamPolicy::Interactive,
    }
}

fn parse_dream_status(value: &str) -> DreamStatus {
    match value {
        "completed" => DreamStatus::Completed,
        "failed" => DreamStatus::Failed,
        "canceled" => DreamStatus::Canceled,
        _ => DreamStatus::Running,
    }
}

/// Transitive supersession chain (both directions) containing `id`.
fn supersession_chain(conn: &Connection, id: &str) -> anyhow::Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT from_id, to_id FROM gbrain_links WHERE kind = ?1")?;
    let mut adj: HashMap<String, Vec<String>> = HashMap::new();
    for row in stmt.query_map(params![SUPERSEDES], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })? {
        let (from, to) = row?;
        adj.entry(from.clone()).or_default().push(to.clone());
        adj.entry(to).or_default().push(from);
    }
    let mut seen = vec![id.to_string()];
    let mut stack = vec![id.to_string()];
    while let Some(node) = stack.pop() {
        if let Some(next) = adj.get(&node) {
            for n in next {
                if !seen.contains(n) {
                    seen.push(n.clone());
                    stack.push(n.clone());
                }
            }
        }
    }
    Ok(seen)
}

fn row_to_fact(row: &rusqlite::Row<'_>) -> rusqlite::Result<TemporalFact> {
    let kind: String = row.get(1)?;
    let value: Option<String> = row.get(2)?;
    let metadata: String = row.get(6)?;
    let valid_at: String = row.get(7)?;
    let invalid_at: Option<String> = row.get(8)?;
    let ingested_at: String = row.get(9)?;
    let expired_at: Option<String> = row.get(10)?;
    let provenance: String = row.get(14)?;
    let created_at: String = row.get(15)?;
    let updated_at: String = row.get(16)?;
    Ok(TemporalFact {
        id: row.get(0)?,
        scope: parse_scope(&kind, value),
        subject: row.get(3)?,
        text: row.get(4)?,
        content_hash: row.get(5)?,
        metadata: serde_json::from_str(&metadata).unwrap_or(Value::Null),
        valid_at: parse_time(&valid_at).unwrap_or(OffsetDateTime::UNIX_EPOCH),
        invalid_at: invalid_at.and_then(|s| parse_time(&s).ok()),
        ingested_at: parse_time(&ingested_at).unwrap_or(OffsetDateTime::UNIX_EPOCH),
        expired_at: expired_at.and_then(|s| parse_time(&s).ok()),
        supersedes: row.get(11)?,
        superseded_by: row.get(12)?,
        supersession_reason: row.get(13)?,
        provenance: serde_json::from_str(&provenance).unwrap_or_default(),
        created_at: parse_time(&created_at).unwrap_or(OffsetDateTime::UNIX_EPOCH),
        updated_at: parse_time(&updated_at).unwrap_or(OffsetDateTime::UNIX_EPOCH),
    })
}

fn parse_scope(kind: &str, value: Option<String>) -> MemoryScope {
    match kind {
        "global" => MemoryScope::Global,
        "user" => MemoryScope::User(value.unwrap_or_default()),
        "workspace" => MemoryScope::Workspace(value.unwrap_or_default()),
        "project" => MemoryScope::Project(value.unwrap_or_default()),
        "thread" => MemoryScope::Thread(value.unwrap_or_default()),
        _ => MemoryScope::Global,
    }
}

// --------------------------------------------------------------------------- //
// Pure helpers
// --------------------------------------------------------------------------- //

/// Detect contradictions: pairs of *transaction-current* facts about the same
/// subject whose valid intervals overlap and that are not in a supersession
/// relationship. Returns index pairs into `facts`.
pub fn detect_contradictions(facts: &[TemporalFact], now: OffsetDateTime) -> Vec<(usize, usize)> {
    let mut pairs = Vec::new();
    for i in 0..facts.len() {
        for j in (i + 1)..facts.len() {
            let a = &facts[i];
            let b = &facts[j];
            let (Some(sa), Some(sb)) = (a.subject.as_ref(), b.subject.as_ref()) else {
                continue;
            };
            if sa != sb {
                continue;
            }
            // Both records must still exist (not retracted).
            if !a.transaction_visible(now) || !b.transaction_visible(now) {
                continue;
            }
            if is_supersession(a, b) {
                continue;
            }
            if valid_intervals_overlap(a, b) {
                pairs.push((i, j));
            }
        }
    }
    pairs
}

fn is_supersession(a: &TemporalFact, b: &TemporalFact) -> bool {
    a.superseded_by.as_deref() == Some(b.id.as_str())
        || b.superseded_by.as_deref() == Some(a.id.as_str())
        || a.supersedes.as_deref() == Some(b.id.as_str())
        || b.supersedes.as_deref() == Some(a.id.as_str())
}

fn valid_intervals_overlap(a: &TemporalFact, b: &TemporalFact) -> bool {
    let a_end = a.invalid_at.unwrap_or(OffsetDateTime::new_utc(
        time::Date::MAX,
        time::Time::MIDNIGHT,
    ));
    let b_end = b.invalid_at.unwrap_or(OffsetDateTime::new_utc(
        time::Date::MAX,
        time::Time::MIDNIGHT,
    ));
    a.valid_at < b_end && b.valid_at < a_end
}

fn canonical_pair<'a>(a: &'a str, b: &'a str) -> (&'a str, &'a str) {
    if a <= b { (a, b) } else { (b, a) }
}

// --------------------------------------------------------------------------- //
// Record mapping (bi-temporal fact <-> generic MemoryRecord)
// --------------------------------------------------------------------------- //

pub fn fact_to_record(fact: &TemporalFact) -> MemoryRecord {
    let now = OffsetDateTime::now_utc();
    let mut metadata = match &fact.metadata {
        Value::Object(_) => fact.metadata.clone(),
        _ => json!({}),
    };
    if let Value::Object(map) = &mut metadata {
        map.insert("subject".into(), json!(fact.subject));
        map.insert("validAt".into(), json!(format_time(fact.valid_at)));
        map.insert("invalidAt".into(), json!(fact.invalid_at.map(format_time)));
        map.insert("ingestedAt".into(), json!(format_time(fact.ingested_at)));
        map.insert("expiredAt".into(), json!(fact.expired_at.map(format_time)));
        map.insert("supersedes".into(), json!(fact.supersedes));
        map.insert("supersededBy".into(), json!(fact.superseded_by));
        map.insert("supersessionReason".into(), json!(fact.supersession_reason));
        map.insert("provenance".into(), json!(fact.provenance));
        map.insert("status".into(), json!(fact.status(now).as_str()));
    }
    MemoryRecord {
        id: Some(fact.id.clone()),
        scope: fact.scope.clone(),
        text: fact.text.clone(),
        content_hash: Some(fact.content_hash.clone()),
        metadata,
        usage: None,
        deleted: fact.expired_at.is_some(),
        created_at: fact.created_at,
        updated_at: fact.updated_at,
    }
}

fn capture_input_from_record(record: MemoryRecord) -> CaptureInput {
    let meta = &record.metadata;
    let read_time = |keys: &[&str]| -> Option<OffsetDateTime> {
        for key in keys {
            if let Some(s) = meta.get(key).and_then(|v| v.as_str())
                && let Ok(dt) = crate::model::parse_flexible(s)
            {
                return Some(dt);
            }
        }
        None
    };
    let subject = meta
        .get("subject")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let provenance = meta
        .get("provenance")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let supersedes = meta
        .get("supersedes")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let supersession_reason = meta
        .get("supersessionReason")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    CaptureInput {
        scope: record.scope,
        subject,
        text: record.text,
        metadata: record.metadata.clone(),
        valid_at: read_time(&["validAt", "valid_at", "timestamp"]),
        invalid_at: read_time(&["invalidAt", "invalid_at"]),
        ingested_at: read_time(&["ingestedAt", "ingested_at"]),
        provenance,
        supersedes,
        supersession_reason,
    }
}

fn snippet(text: &str) -> String {
    const MAX: usize = 180;
    if text.chars().count() <= MAX {
        text.to_string()
    } else {
        let mut out = text.chars().take(MAX).collect::<String>();
        out.push_str("...");
        out
    }
}

/// Human-readable label for a fact's lifecycle (used by render + tools).
pub fn status_label(fact: &TemporalFact, now: OffsetDateTime) -> &'static str {
    match fact.status(now) {
        FactStatus::Current => "current",
        FactStatus::Superseded => "superseded",
        FactStatus::Invalidated => "invalidated",
        FactStatus::Retracted => "retracted",
    }
}
