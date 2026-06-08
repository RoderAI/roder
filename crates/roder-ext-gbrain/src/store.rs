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

use crate::dream::{DreamMode, DreamPolicy, DreamStatus, GraphMaterializationStats};
use crate::embed::{Embedder, Embedding};
use crate::import::{DedupeMode, ImportBatchParams, ImportBatchResult, load_import_batch};
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
const DREAM_ALGORITHM_VERSION: &str = "phase84-materialized-v2";

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QueryFeedbackInput {
    pub scope: Option<MemoryScope>,
    pub question: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question_kind: Option<String>,
    #[serde(default)]
    pub used_nodes: Vec<String>,
    #[serde(default)]
    pub used_cards: Vec<String>,
    #[serde(default)]
    pub used_events: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    pub tool_call_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer_length: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eval_result_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QueryFeedbackRow {
    pub id: String,
    pub scope_id: Option<String>,
    pub question: String,
    pub question_kind: Option<String>,
    pub used_nodes: Vec<String>,
    pub used_cards: Vec<String>,
    pub used_events: Vec<String>,
    pub duration_ms: Option<u64>,
    pub tool_call_count: usize,
    pub stop_reason: Option<String>,
    pub answer_length: Option<usize>,
    pub response_hash: Option<String>,
    pub eval_result_id: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MemorySnapshotReport {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_snapshot_high_watermark: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_dream_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_ontology_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derived_snapshot_version: Option<String>,
    pub graph_node_count: usize,
    pub graph_edge_count: usize,
    pub evidence_card_count: usize,
    pub query_feedback_count: usize,
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

    pub fn db_path(&self) -> PathBuf {
        self.base_path.join("gbrain.sqlite3")
    }
}

impl MemoryStoreFactory for GbrainStoreFactory {
    fn id(&self) -> MemoryStoreId {
        "gbrain-bitemporal".to_string()
    }

    fn create(&self) -> Arc<dyn MemoryStore> {
        Arc::new(
            GbrainStore::open(self.db_path(), Embedder::new(self.provider.clone()))
                .expect("open gbrain store"),
        )
    }

    fn storage_path(&self) -> Option<PathBuf> {
        Some(self.db_path())
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
        let embedding = self.embedder.embed_document(&input.text).await;
        self.with_conn(|conn| capture_blocking(conn, input, embedding))
    }

    /// Hybrid recall over the snapshot defined by `params.as_of`.
    pub async fn recall(&self, params: RecallParams) -> anyhow::Result<RecallResult> {
        // De-meta focal query (roadmap/91, GBRAIN_FOCAL_QUERY=1): for "walk me
        // through the evidence for <X>" meta-questions, retrieve on the extracted
        // focus <X> so the generic meta-verbs don't pull a topically-similar WRONG
        // event. Affects retrieval ONLY (embedding + lexical); the synthesizer still
        // sees the original question. No-op for plain questions.
        let mut params = params;
        if std::env::var("GBRAIN_NO_FOCAL_QUERY").is_err()
            && let Some(focus) = crate::retrieval::focal_retrieval_query(&params.query)
        {
            params.query = focus;
        }
        let query_embedding = self.embedder.embed_query(&params.query).await;
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

    /// Import raw facts from a batch payload. JSONL file/string input and
    /// markdown corpus directory input are supported.
    pub async fn import_batch(
        &self,
        params: ImportBatchParams,
    ) -> anyhow::Result<ImportBatchResult> {
        let ImportBatchParams {
            input,
            format,
            scope,
            source,
            dedupe,
            dream_after_import,
            metadata,
        } = params;
        let dream_after_import_mode = dream_after_import
            .as_deref()
            .map(parse_import_dream_mode)
            .transpose()?
            .flatten();
        let loaded = load_import_batch(&input, &format)?;
        let source_path = loaded.source_path.clone();
        let source_hash = loaded.source_hash.clone();
        let run_id = uuid::Uuid::new_v4().to_string();
        let started_at = OffsetDateTime::now_utc();
        self.with_conn(|conn| {
            let scope_id = ensure_scope(conn, &scope)?;
            conn.execute(
                "INSERT INTO gbrain_import_runs(id, scope_id, source_path, source_hash, started_at, status, metadata)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'running', ?6)",
                params![
                    run_id,
                    scope_id,
                    source_path.as_deref(),
                    source_hash.as_str(),
                    format_time(started_at),
                    serde_json::to_string(&json!({
                        "source": source,
                        "format": format,
                        "dedupe": dedupe.as_str(),
                        "dream_after_import": dream_after_import,
                        "metadata": metadata,
                    }))?,
                ],
            )?;
            Ok(())
        })?;

        let mut total = 0usize;
        let mut inserted = 0usize;
        let mut skipped_duplicates = 0usize;
        let errors = loaded.errors;
        let mut fact_ids = Vec::new();

        for loaded_record in loaded.records {
            total += 1;
            let record = loaded_record.record;
            let record_hash = content_hash(&record.text);
            let source_id = record.source_id.clone();
            let duplicate = self.with_conn(|conn| {
                import_duplicate_exists(conn, &scope, dedupe, source_id.as_deref(), &record_hash)
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
                if let Some(source) = &source {
                    map.insert("source".into(), json!(source));
                }
                if let Some(thread_id) = &record.thread_id {
                    map.insert("thread_id".into(), json!(thread_id));
                }
                if let Some(slug) = &record.slug {
                    map.insert("slug".into(), json!(slug));
                }
                map.insert("import_run_id".into(), json!(run_id));
                map.insert("import_line".into(), json!(loaded_record.line_index + 1));
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

            let mut input = CaptureInput::new(scope.clone(), record.text);
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
                        Some(format!("{errors} import record(s) failed to parse"))
                    },
                    serde_json::to_string(&json!({
                        "source": source,
                        "format": format,
                        "dedupe": dedupe.as_str(),
                        "total": total,
                        "inserted": inserted,
                        "skipped_duplicates": skipped_duplicates,
                        "errors": errors,
                        "dream_after_import": dream_after_import,
                        "metadata": metadata,
                    }))?,
                    run_id,
                ],
            )?;
            Ok(())
        })?;

        let dream_report = if errors == 0 {
            if let Some(mode) = dream_after_import_mode {
                Some(
                    self.dream(DreamParams {
                        mode,
                        scope: scope.clone(),
                        since: None,
                        run_policy: DreamPolicy::Import,
                        workers: 1,
                        dry_run: false,
                        cancellation_token: None,
                        reasoner_model: None,
                    })
                    .await
                    .with_context(|| {
                        format!(
                            "post-import dream failed for import run {run_id} with mode {}",
                            mode.as_str()
                        )
                    })?,
                )
            } else {
                None
            }
        } else {
            None
        };
        let (node_count, edge_count) =
            self.with_conn(|conn| count_materialized_graph_rows(conn, &scope))?;
        let dream_metadata = dream_report.as_ref().map(|report| {
            json!({
                "dream_run_id": report.id,
                "dream_mode": report.mode.as_str(),
                "dream_algorithm_version": report.algorithm_version,
                "derived_statement_count": report.derived_statement_count,
                "derived_event_count": report.derived_event_count,
            })
        });
        let dream_run_id = dream_report.as_ref().map(|report| report.id.as_str());
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO gbrain_import_manifest(
                    id, import_run_id, source_hash, source_path, source_prefix, corpus_prefix,
                    replacement_policy, fact_count, statement_count, node_count, edge_count,
                    added_at, metadata
                 )
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    uuid::Uuid::new_v4().to_string(),
                    run_id,
                    source_hash,
                    source_path.as_deref(),
                    source.as_deref(),
                    source_path.as_deref().or(source.as_deref()),
                    dedupe.as_str(),
                    inserted as i64,
                    dream_report
                        .as_ref()
                        .map(|report| report.derived_statement_count)
                        .unwrap_or(0) as i64,
                    node_count as i64,
                    edge_count as i64,
                    format_time(OffsetDateTime::now_utc()),
                    serde_json::to_string(&json!({
                        "source": source,
                        "format": format,
                        "scope_id": scope.stable_id(),
                        "total": total,
                        "inserted": inserted,
                        "skipped_duplicates": skipped_duplicates,
                        "errors": errors,
                        "dream_after_import": dream_after_import,
                        "dream_run_id": dream_run_id,
                        "dream": dream_metadata,
                        "metadata": metadata,
                    }))?,
                ],
            )?;
            Ok(())
        })?;

        Ok(ImportBatchResult {
            run_id,
            status: status.to_string(),
            scope_id: scope.stable_id(),
            source,
            format,
            dedupe,
            total,
            inserted,
            skipped_duplicates,
            errors,
            fact_ids,
        })
    }

    /// Run explicit at-rest dream maintenance. The phase-84 runner records the
    /// ledger, rebuilds links, and materializes graph/evidence rows for
    /// Obsidian/app-server visualization before query-time retrieval starts.
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
                    DREAM_ALGORITHM_VERSION,
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
        let mut materialized = GraphMaterializationStats::default();
        if !params.dry_run && matches!(params.mode, DreamMode::Refine | DreamMode::Full) {
            let stats = self.consolidate(Some(params.scope.clone())).await?;
            derived_event_count = stats.supersession_links + stats.contradiction_links;
            invalidated_event_count = stats.contradiction_links;
        }
        if !params.dry_run {
            materialized = self.with_conn(|conn| {
                crate::dream::materialize_dream_graph(conn, &params.scope, &run_id, started_at)
            })?;
            derived_event_count += materialized.derived_event_count();
        }
        let derived_statement_count = materialized.derived_statement_count();

        let finished_at = OffsetDateTime::now_utc();
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE gbrain_dream_runs
                 SET finished_at = ?1, status = 'completed', derived_statement_count = ?2,
                     derived_event_count = ?3, invalidated_event_count = ?4
                 WHERE id = ?5",
                params![
                    format_time(finished_at),
                    derived_statement_count as i64,
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
            algorithm_version: DREAM_ALGORITHM_VERSION.into(),
            run_policy: params.run_policy,
            started_at,
            finished_at: Some(finished_at),
            workers,
            input_fact_count,
            derived_statement_count,
            derived_event_count,
            invalidated_event_count,
            reasoner_model: params.reasoner_model,
            error: None,
        })
    }

    pub async fn dream_status(&self, run_id: &str) -> anyhow::Result<Option<DreamRunReport>> {
        self.with_conn(|conn| load_dream_run(conn, run_id))
    }

    pub async fn append_query_feedback(
        &self,
        input: QueryFeedbackInput,
    ) -> anyhow::Result<QueryFeedbackRow> {
        self.with_conn(|conn| append_query_feedback(conn, input))
    }

    pub async fn load_query_feedback(
        &self,
        scope: Option<MemoryScope>,
    ) -> anyhow::Result<Vec<QueryFeedbackRow>> {
        self.with_conn(|conn| load_query_feedback(conn, scope.as_ref()))
    }

    pub async fn memory_snapshot(
        &self,
        scope: Option<MemoryScope>,
    ) -> anyhow::Result<MemorySnapshotReport> {
        self.with_conn(|conn| load_memory_snapshot(conn, scope.as_ref()))
    }

    pub async fn find_dream_start_nodes(
        &self,
        query: &str,
        scope: Option<MemoryScope>,
        node_kinds: &[String],
        limit: usize,
    ) -> anyhow::Result<Vec<Value>> {
        self.with_conn(|conn| {
            find_dream_start_nodes(conn, query, scope.as_ref(), node_kinds, limit)
        })
    }

    pub async fn expand_dream_neighbors(
        &self,
        node_id: &str,
        edge_kinds: &[String],
        depth: usize,
    ) -> anyhow::Result<Vec<Value>> {
        self.with_conn(|conn| expand_dream_neighbors(conn, node_id, edge_kinds, depth))
    }

    pub async fn find_dream_paths(
        &self,
        source_node_id: &str,
        target_node_id: &str,
        relation_filter: &[String],
        budget: usize,
    ) -> anyhow::Result<Vec<Value>> {
        self.with_conn(|conn| {
            find_dream_paths(
                conn,
                source_node_id,
                target_node_id,
                relation_filter,
                budget,
            )
        })
    }

    pub async fn explain_dream_node(&self, node_id: &str) -> anyhow::Result<Option<Value>> {
        self.with_conn(|conn| explain_dream_node(conn, node_id))
    }

    pub async fn dream_node_community(
        &self,
        node_id: Option<&str>,
        community_id: Option<&str>,
        include_members: bool,
    ) -> anyhow::Result<Option<Value>> {
        self.with_conn(|conn| dream_node_community(conn, node_id, community_id, include_members))
    }

    /// In-place update of a fact's text/metadata (re-embeds). Used by the
    /// generic `MemoryStore::put` update path.
    async fn update_in_place(&self, id: &str, text: String, metadata: Value) -> anyhow::Result<()> {
        let embedding = self.embedder.embed_document(&text).await;
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

fn append_query_feedback(
    conn: &Connection,
    input: QueryFeedbackInput,
) -> anyhow::Result<QueryFeedbackRow> {
    let id = uuid::Uuid::new_v4().to_string();
    let scope_id = input
        .scope
        .as_ref()
        .map(|scope| ensure_scope(conn, scope))
        .transpose()?;
    let created_at = OffsetDateTime::now_utc();
    conn.execute(
        "INSERT INTO gbrain_query_feedback(
            id, scope_id, question, question_kind, used_nodes, used_cards, used_events,
            duration_ms, tool_call_count, stop_reason, answer_length, response_hash,
            eval_result_id, created_at
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            id,
            scope_id,
            input.question,
            input.question_kind,
            serde_json::to_string(&input.used_nodes)?,
            serde_json::to_string(&input.used_cards)?,
            serde_json::to_string(&input.used_events)?,
            input.duration_ms.map(|value| value as i64),
            input.tool_call_count as i64,
            input.stop_reason,
            input.answer_length.map(|value| value as i64),
            input.response_hash,
            input.eval_result_id,
            format_time(created_at),
        ],
    )?;
    load_query_feedback_by_id(conn, &id)
}

fn load_query_feedback(
    conn: &Connection,
    scope: Option<&MemoryScope>,
) -> anyhow::Result<Vec<QueryFeedbackRow>> {
    let scope_id = scope.map(MemoryScope::stable_id);
    let mut stmt = if scope_id.is_some() {
        conn.prepare(
            "SELECT id, scope_id, question, question_kind, used_nodes, used_cards, used_events,
                    duration_ms, tool_call_count, stop_reason, answer_length, response_hash,
                    eval_result_id, created_at
             FROM gbrain_query_feedback
             WHERE scope_id = ?1
             ORDER BY created_at, id",
        )?
    } else {
        conn.prepare(
            "SELECT id, scope_id, question, question_kind, used_nodes, used_cards, used_events,
                    duration_ms, tool_call_count, stop_reason, answer_length, response_hash,
                    eval_result_id, created_at
             FROM gbrain_query_feedback
             ORDER BY created_at, id",
        )?
    };
    let rows = if let Some(scope_id) = scope_id {
        stmt.query_map(params![scope_id], query_feedback_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?
    } else {
        stmt.query_map([], query_feedback_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    Ok(rows)
}

fn load_query_feedback_by_id(conn: &Connection, id: &str) -> anyhow::Result<QueryFeedbackRow> {
    Ok(conn.query_row(
        "SELECT id, scope_id, question, question_kind, used_nodes, used_cards, used_events,
                duration_ms, tool_call_count, stop_reason, answer_length, response_hash,
                eval_result_id, created_at
         FROM gbrain_query_feedback
         WHERE id = ?1",
        params![id],
        query_feedback_from_row,
    )?)
}

fn query_feedback_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<QueryFeedbackRow> {
    let used_nodes: String = row.get(4)?;
    let used_cards: String = row.get(5)?;
    let used_events: String = row.get(6)?;
    let duration_ms: Option<i64> = row.get(7)?;
    let tool_call_count: i64 = row.get(8)?;
    let answer_length: Option<i64> = row.get(10)?;
    let created_at: String = row.get(13)?;
    Ok(QueryFeedbackRow {
        id: row.get(0)?,
        scope_id: row.get(1)?,
        question: row.get(2)?,
        question_kind: row.get(3)?,
        used_nodes: serde_json::from_str(&used_nodes).unwrap_or_default(),
        used_cards: serde_json::from_str(&used_cards).unwrap_or_default(),
        used_events: serde_json::from_str(&used_events).unwrap_or_default(),
        duration_ms: duration_ms.map(|value| value.max(0) as u64),
        tool_call_count: tool_call_count.max(0) as usize,
        stop_reason: row.get(9)?,
        answer_length: answer_length.map(|value| value.max(0) as usize),
        response_hash: row.get(11)?,
        eval_result_id: row.get(12)?,
        created_at: parse_time(&created_at).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(
                13,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    err.to_string(),
                )),
            )
        })?,
    })
}

fn load_memory_snapshot(
    conn: &Connection,
    scope: Option<&MemoryScope>,
) -> anyhow::Result<MemorySnapshotReport> {
    let scope_id = scope.map(MemoryScope::stable_id);
    let raw_snapshot_high_watermark: Option<String> = match &scope_id {
        Some(scope_id) => conn.query_row(
            "SELECT MAX(ingested_at) FROM gbrain_facts WHERE scope_id = ?1",
            params![scope_id],
            |row| row.get(0),
        )?,
        None => conn.query_row("SELECT MAX(ingested_at) FROM gbrain_facts", [], |row| {
            row.get(0)
        })?,
    };
    let selected_dream_run: Option<(String, String)> = match &scope_id {
        Some(scope_id) => conn
            .query_row(
                "SELECT id, algorithm_version
                 FROM gbrain_dream_runs
                 WHERE scope_id = ?1 AND status = 'completed'
                 ORDER BY COALESCE(finished_at, started_at) DESC, id DESC
                 LIMIT 1",
                params![scope_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?,
        None => conn
            .query_row(
                "SELECT id, algorithm_version
                 FROM gbrain_dream_runs
                 WHERE status = 'completed'
                 ORDER BY COALESCE(finished_at, started_at) DESC, id DESC
                 LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?,
    };
    let selected_ontology_version: Option<String> = conn
        .query_row(
            "SELECT version
             FROM gbrain_ontology_nodes
             WHERE active = 1
             ORDER BY created_at DESC, id DESC
             LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;

    let graph_node_count = count_active_by_scope(conn, "gbrain_nodes", &scope_id)?;
    let evidence_card_count = count_active_by_scope(conn, "gbrain_evidence_cards", &scope_id)?;
    let graph_edge_count = match &scope_id {
        Some(scope_id) => conn.query_row(
            "SELECT COUNT(DISTINCT e.id)
             FROM gbrain_edges e
             JOIN gbrain_nodes source ON source.id = e.source_node_id
             JOIN gbrain_nodes target ON target.id = e.target_node_id
             WHERE e.active = 1 AND (source.scope_id = ?1 OR target.scope_id = ?1)",
            params![scope_id],
            |row| row.get::<_, i64>(0),
        )?,
        None => conn.query_row(
            "SELECT COUNT(*) FROM gbrain_edges WHERE active = 1",
            [],
            |row| row.get::<_, i64>(0),
        )?,
    }
    .max(0) as usize;
    let query_feedback_count = match &scope_id {
        Some(scope_id) => conn.query_row(
            "SELECT COUNT(*) FROM gbrain_query_feedback WHERE scope_id = ?1",
            params![scope_id],
            |row| row.get::<_, i64>(0),
        )?,
        None => conn.query_row("SELECT COUNT(*) FROM gbrain_query_feedback", [], |row| {
            row.get::<_, i64>(0)
        })?,
    }
    .max(0) as usize;

    Ok(MemorySnapshotReport {
        raw_snapshot_high_watermark,
        selected_dream_run_id: selected_dream_run.as_ref().map(|(id, _)| id.clone()),
        selected_ontology_version,
        derived_snapshot_version: selected_dream_run.map(|(_, version)| version),
        graph_node_count,
        graph_edge_count,
        evidence_card_count,
        query_feedback_count,
    })
}

fn count_active_by_scope(
    conn: &Connection,
    table: &str,
    scope_id: &Option<String>,
) -> anyhow::Result<usize> {
    let count: i64 = match scope_id {
        Some(scope_id) => conn.query_row(
            &format!("SELECT COUNT(*) FROM {table} WHERE scope_id = ?1 AND active = 1"),
            params![scope_id],
            |row| row.get(0),
        )?,
        None => conn.query_row(
            &format!("SELECT COUNT(*) FROM {table} WHERE active = 1"),
            [],
            |row| row.get(0),
        )?,
    };
    Ok(count.max(0) as usize)
}

fn find_dream_start_nodes(
    conn: &Connection,
    query: &str,
    scope: Option<&MemoryScope>,
    node_kinds: &[String],
    limit: usize,
) -> anyhow::Result<Vec<Value>> {
    let scope_id = scope.map(MemoryScope::stable_id);
    let mut stmt = conn.prepare(
        "SELECT n.id, n.label, n.node_kind, n.source_artifact, n.source_location,
                n.source_span, n.source_fact_id, n.created_by_run_id, n.confidence,
                f.text, n.scope_id
         FROM gbrain_nodes n
         LEFT JOIN gbrain_facts f ON f.id = n.source_fact_id
         WHERE n.active = 1
         ORDER BY n.created_at DESC
         LIMIT 2000",
    )?;
    let mut observations = Vec::new();
    for row in stmt.query_map([], dream_node_candidate_from_row)? {
        let candidate = row?;
        if let Some(scope_id) = &scope_id
            && candidate.scope_id.as_deref() != Some(scope_id.as_str())
        {
            continue;
        }
        if !node_kinds.is_empty()
            && !node_kinds
                .iter()
                .any(|kind| kind.eq_ignore_ascii_case(&candidate.node_kind))
        {
            continue;
        }
        let score = lexical_score(
            query,
            &[
                candidate.label.as_str(),
                candidate.source_artifact.as_deref().unwrap_or_default(),
                candidate.text.as_deref().unwrap_or_default(),
            ],
        );
        if score == 0 && !query.trim().is_empty() {
            continue;
        }
        observations.push((score, dream_node_observation(conn, &candidate)?));
    }
    observations.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    Ok(observations
        .into_iter()
        .take(limit.max(1))
        .map(|(_, observation)| observation)
        .collect())
}

fn expand_dream_neighbors(
    conn: &Connection,
    node_id: &str,
    edge_kinds: &[String],
    depth: usize,
) -> anyhow::Result<Vec<Value>> {
    let max_depth = depth.clamp(1, 4);
    let mut frontier = vec![node_id.to_string()];
    let mut seen_nodes = HashSet::from([node_id.to_string()]);
    let mut seen_edges = HashSet::new();
    let mut observations = Vec::new();
    for current_depth in 1..=max_depth {
        let mut next = Vec::new();
        for current in &frontier {
            for edge in dream_edges_for_node(conn, current, edge_kinds)? {
                let edge_id = edge["id"].as_str().unwrap_or_default().to_string();
                if !seen_edges.insert(edge_id) {
                    continue;
                }
                let source = edge["sourceNodeId"].as_str().unwrap_or_default();
                let target = edge["targetNodeId"].as_str().unwrap_or_default();
                let neighbor_id = if source == current { target } else { source };
                let neighbor = load_dream_node(conn, neighbor_id)?;
                observations.push(json!({
                    "observationType": "dream_neighbor",
                    "depth": current_depth,
                    "edge": edge,
                    "node": neighbor,
                    "trace": {
                        "source": "gbrain_edges",
                        "readOnly": true,
                        "fallback": false,
                    }
                }));
                if seen_nodes.insert(neighbor_id.to_string()) {
                    next.push(neighbor_id.to_string());
                }
            }
        }
        frontier = next;
        if frontier.is_empty() {
            break;
        }
    }
    Ok(observations)
}

fn find_dream_paths(
    conn: &Connection,
    source_node_id: &str,
    target_node_id: &str,
    relation_filter: &[String],
    budget: usize,
) -> anyhow::Result<Vec<Value>> {
    let mut observations = Vec::new();
    for edge in dream_edges_for_node(conn, source_node_id, relation_filter)? {
        let source = edge["sourceNodeId"].as_str().unwrap_or_default();
        let target = edge["targetNodeId"].as_str().unwrap_or_default();
        if source == target_node_id || target == target_node_id {
            observations.push(json!({
                "observationType": "dream_path",
                "pathKind": "direct",
                "nodes": [load_dream_node(conn, source_node_id)?, load_dream_node(conn, target_node_id)?],
                "edges": [edge],
                "trace": {"source": "gbrain_edges", "readOnly": true, "fallback": false}
            }));
        }
    }
    if !observations.is_empty() {
        return Ok(observations.into_iter().take(budget.max(1)).collect());
    }

    for first_edge in dream_edges_for_node(conn, source_node_id, relation_filter)? {
        let first_source = first_edge["sourceNodeId"].as_str().unwrap_or_default();
        let first_target = first_edge["targetNodeId"].as_str().unwrap_or_default();
        let midpoint = if first_source == source_node_id {
            first_target
        } else {
            first_source
        };
        for second_edge in dream_edges_for_node(conn, midpoint, relation_filter)? {
            let second_source = second_edge["sourceNodeId"].as_str().unwrap_or_default();
            let second_target = second_edge["targetNodeId"].as_str().unwrap_or_default();
            if second_source == target_node_id || second_target == target_node_id {
                observations.push(json!({
                    "observationType": "dream_path",
                    "pathKind": "two_hop",
                    "nodes": [
                        load_dream_node(conn, source_node_id)?,
                        load_dream_node(conn, midpoint)?,
                        load_dream_node(conn, target_node_id)?
                    ],
                    "edges": [first_edge, second_edge],
                    "trace": {"source": "gbrain_edges", "readOnly": true, "fallback": false}
                }));
                if observations.len() >= budget.max(1) {
                    return Ok(observations);
                }
            }
        }
    }
    Ok(observations)
}

fn explain_dream_node(conn: &Connection, node_id: &str) -> anyhow::Result<Option<Value>> {
    let Some(node) = load_dream_node_optional(conn, node_id)? else {
        return Ok(None);
    };
    let evidence_cards = evidence_cards_for_node(conn, node_id)?;
    let edges = dream_edges_for_node(conn, node_id, &[])?;
    Ok(Some(json!({
        "observationType": "dream_node_explanation",
        "node": node,
        "evidenceCards": evidence_cards,
        "edges": edges,
        "trace": {
            "source": "gbrain_nodes",
            "readOnly": true,
            "fallback": false,
        }
    })))
}

fn dream_node_community(
    conn: &Connection,
    node_id: Option<&str>,
    community_id: Option<&str>,
    include_members: bool,
) -> anyhow::Result<Option<Value>> {
    if let Some(community_id) = community_id {
        let community = conn
            .query_row(
                "SELECT id, scope_id, version, label, cohesion_score, dream_run_id
                 FROM gbrain_communities
                 WHERE id = ?1 AND active = 1",
                params![community_id],
                |row| {
                    Ok(json!({
                        "id": row.get::<_, String>(0)?,
                        "scopeId": row.get::<_, String>(1)?,
                        "version": row.get::<_, String>(2)?,
                        "label": row.get::<_, Option<String>>(3)?,
                        "cohesionScore": row.get::<_, f64>(4)?,
                        "dreamRunId": row.get::<_, Option<String>>(5)?,
                    }))
                },
            )
            .optional()?;
        if let Some(community) = community {
            return Ok(Some(json!({
                "observationType": "dream_community",
                "community": community,
                "members": if include_members {
                    community_members(conn, community_id)?
                } else {
                    Vec::<Value>::new()
                },
                "trace": {"source": "gbrain_communities", "readOnly": true, "fallback": false}
            })));
        }
    }
    if let Some(node_id) = node_id {
        let Some(node) = load_dream_node_optional(conn, node_id)? else {
            return Ok(None);
        };
        let neighbors = expand_dream_neighbors(conn, node_id, &[], 1)?;
        return Ok(Some(json!({
            "observationType": "dream_local_community",
            "community": {
                "id": format!("local:{node_id}"),
                "label": node["label"],
                "communityKind": "local_neighborhood",
            },
            "members": if include_members { neighbors } else { Vec::<Value>::new() },
            "trace": {"source": "gbrain_edges", "readOnly": true, "fallback": false}
        })));
    }
    Ok(None)
}

#[derive(Debug)]
struct DreamNodeCandidate {
    id: String,
    label: String,
    node_kind: String,
    source_artifact: Option<String>,
    source_location: Option<String>,
    source_span: Option<String>,
    source_fact_id: Option<String>,
    dream_run_id: Option<String>,
    confidence: String,
    text: Option<String>,
    scope_id: Option<String>,
}

fn dream_node_candidate_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DreamNodeCandidate> {
    Ok(DreamNodeCandidate {
        id: row.get(0)?,
        label: row.get(1)?,
        node_kind: row.get(2)?,
        source_artifact: row.get(3)?,
        source_location: row.get(4)?,
        source_span: row.get(5)?,
        source_fact_id: row.get(6)?,
        dream_run_id: row.get(7)?,
        confidence: row.get(8)?,
        text: row.get(9)?,
        scope_id: row.get(10)?,
    })
}

fn dream_node_observation(conn: &Connection, node: &DreamNodeCandidate) -> anyhow::Result<Value> {
    Ok(json!({
        "observationType": "dream_start_node",
        "node": dream_node_json(node),
        "evidenceCards": evidence_cards_for_source_fact(conn, node.source_fact_id.as_deref())?,
        "trace": {
            "source": "gbrain_nodes",
            "readOnly": true,
            "fallback": false,
            "dreamRunId": node.dream_run_id,
        }
    }))
}

fn dream_node_json(node: &DreamNodeCandidate) -> Value {
    json!({
        "id": node.id,
        "label": node.label,
        "kind": node.node_kind,
        "sourceArtifact": node.source_artifact,
        "sourceLocation": node.source_location,
        "sourceSpan": node.source_span,
        "sourceFactId": node.source_fact_id,
        "dreamRunId": node.dream_run_id,
        "confidence": node.confidence,
    })
}

fn load_dream_node(conn: &Connection, node_id: &str) -> anyhow::Result<Value> {
    load_dream_node_optional(conn, node_id)?
        .ok_or_else(|| anyhow::anyhow!("dream node not found: {node_id}"))
}

fn load_dream_node_optional(conn: &Connection, node_id: &str) -> anyhow::Result<Option<Value>> {
    conn.query_row(
        "SELECT id, label, node_kind, source_artifact, source_location, source_span,
                source_fact_id, created_by_run_id, confidence, NULL, scope_id
         FROM gbrain_nodes
         WHERE id = ?1 AND active = 1",
        params![node_id],
        |row| dream_node_candidate_from_row(row).map(|node| dream_node_json(&node)),
    )
    .optional()
    .map_err(Into::into)
}

fn dream_edges_for_node(
    conn: &Connection,
    node_id: &str,
    relation_filter: &[String],
) -> anyhow::Result<Vec<Value>> {
    let mut stmt = conn.prepare(
        "SELECT id, source_node_id, target_node_id, relation, confidence, source_artifact,
                source_location, source_span, evidence_ids, directed, original_direction,
                ontology_edge_id, dream_run_id, valid_at, invalid_at, transaction_at
         FROM gbrain_edges
         WHERE active = 1 AND (source_node_id = ?1 OR target_node_id = ?1)
         ORDER BY created_at DESC
         LIMIT 200",
    )?;
    let mut edges = Vec::new();
    for row in stmt.query_map(params![node_id], dream_edge_from_row)? {
        let edge = row?;
        if !relation_filter.is_empty()
            && !relation_filter.iter().any(|relation| {
                edge["relation"]
                    .as_str()
                    .is_some_and(|value| value.eq_ignore_ascii_case(relation))
            })
        {
            continue;
        }
        edges.push(edge);
    }
    Ok(edges)
}

fn dream_edge_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let evidence_ids: String = row.get(8)?;
    let directed: i64 = row.get(9)?;
    Ok(json!({
        "id": row.get::<_, String>(0)?,
        "sourceNodeId": row.get::<_, String>(1)?,
        "targetNodeId": row.get::<_, String>(2)?,
        "relation": row.get::<_, String>(3)?,
        "confidence": row.get::<_, String>(4)?,
        "sourceArtifact": row.get::<_, Option<String>>(5)?,
        "sourceLocation": row.get::<_, Option<String>>(6)?,
        "sourceSpan": row.get::<_, Option<String>>(7)?,
        "evidenceIds": serde_json::from_str::<Vec<String>>(&evidence_ids).unwrap_or_default(),
        "directed": directed != 0,
        "originalDirection": row.get::<_, Option<String>>(10)?,
        "ontologyEdgeId": row.get::<_, Option<String>>(11)?,
        "dreamRunId": row.get::<_, Option<String>>(12)?,
        "validAt": row.get::<_, Option<String>>(13)?,
        "invalidAt": row.get::<_, Option<String>>(14)?,
        "transactionAt": row.get::<_, Option<String>>(15)?,
    }))
}

fn evidence_cards_for_node(conn: &Connection, node_id: &str) -> anyhow::Result<Vec<Value>> {
    let source_fact_id: Option<String> = conn
        .query_row(
            "SELECT source_fact_id FROM gbrain_nodes WHERE id = ?1",
            params![node_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    evidence_cards_for_source_fact(conn, source_fact_id.as_deref())
}

fn evidence_cards_for_source_fact(
    conn: &Connection,
    source_fact_id: Option<&str>,
) -> anyhow::Result<Vec<Value>> {
    let Some(source_fact_id) = source_fact_id else {
        return Ok(Vec::new());
    };
    let pattern = format!("%\"{source_fact_id}\"%");
    let mut stmt = conn.prepare(
        "SELECT id, title, summary, quote_spans, source_fact_ids, temporal_status,
                neighboring_event_ids, confidence, dream_run_id
         FROM gbrain_evidence_cards
         WHERE active = 1 AND source_fact_ids LIKE ?1
         ORDER BY created_at DESC
         LIMIT 20",
    )?;
    let cards = stmt
        .query_map(params![pattern], evidence_card_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(cards)
}

fn evidence_card_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let quote_spans: String = row.get(3)?;
    let source_fact_ids: String = row.get(4)?;
    let neighboring_event_ids: String = row.get(6)?;
    Ok(json!({
        "id": row.get::<_, String>(0)?,
        "title": row.get::<_, String>(1)?,
        "summary": row.get::<_, String>(2)?,
        "quoteSpans": serde_json::from_str::<Vec<Value>>(&quote_spans).unwrap_or_default(),
        "sourceFactIds": serde_json::from_str::<Vec<String>>(&source_fact_ids).unwrap_or_default(),
        "temporalStatus": row.get::<_, Option<String>>(5)?,
        "neighboringEventIds": serde_json::from_str::<Vec<String>>(&neighboring_event_ids).unwrap_or_default(),
        "confidence": row.get::<_, String>(7)?,
        "dreamRunId": row.get::<_, Option<String>>(8)?,
    }))
}

fn community_members(conn: &Connection, community_id: &str) -> anyhow::Result<Vec<Value>> {
    let mut stmt = conn.prepare(
        "SELECT node_id, membership_score, hub, noise
         FROM gbrain_community_members
         WHERE community_id = ?1
         ORDER BY membership_score DESC, node_id
         LIMIT 100",
    )?;
    let members = stmt
        .query_map(params![community_id], |row| {
            let node_id: String = row.get(0)?;
            Ok(json!({
                "node": load_dream_node(conn, &node_id).unwrap_or(Value::Null),
                "membershipScore": row.get::<_, f64>(1)?,
                "hub": row.get::<_, i64>(2)? != 0,
                "noise": row.get::<_, i64>(3)? != 0,
            }))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(members)
}

fn lexical_score(query: &str, fields: &[&str]) -> usize {
    let haystack = fields.join(" ").to_ascii_lowercase();
    query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|term| term.len() > 2)
        .map(str::to_ascii_lowercase)
        .filter(|term| haystack.contains(term))
        .count()
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
        // Gated recency boost (roadmap/91 P6, GBRAIN_RECENCY=1): for "as of now /
        // what changed / currently believe" questions, nudge the most-recent
        // (correcting) record up BEFORE truncation so it enters top_k instead of the
        // stale original it supersedes. Gated on intent so plain/decision questions
        // are unaffected (a recent amendment must NOT win there).
        if std::env::var("GBRAIN_RECENCY").is_ok()
            && crate::retrieval::recency_intent(&params.query)
        {
            crate::retrieval::apply_recency_boost(&mut scored, now);
        }
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

fn parse_import_dream_mode(value: &str) -> anyhow::Result<Option<DreamMode>> {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "none" | "off" | "false" | "no" => Ok(None),
        "enrich" => Ok(Some(DreamMode::Enrich)),
        "refine" => Ok(Some(DreamMode::Refine)),
        "compact" => Ok(Some(DreamMode::Compact)),
        "full" => Ok(Some(DreamMode::Full)),
        other => {
            anyhow::bail!(
                "unknown dream_after_import mode {other:?}; expected enrich|refine|compact|full"
            )
        }
    }
}

fn count_materialized_graph_rows(
    conn: &Connection,
    scope: &MemoryScope,
) -> anyhow::Result<(usize, usize)> {
    let scope_id = scope.stable_id();
    let node_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM gbrain_nodes WHERE scope_id = ?1",
        params![scope_id],
        |row| row.get(0),
    )?;
    let edge_count: i64 = conn.query_row(
        "SELECT COUNT(DISTINCT e.id)
         FROM gbrain_edges e
         JOIN gbrain_nodes source ON source.id = e.source_node_id
         JOIN gbrain_nodes target ON target.id = e.target_node_id
         WHERE source.scope_id = ?1 OR target.scope_id = ?1",
        params![scope.stable_id()],
        |row| row.get(0),
    )?;
    Ok((node_count.max(0) as usize, edge_count.max(0) as usize))
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
