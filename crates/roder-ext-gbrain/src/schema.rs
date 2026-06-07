//! SQLite schema for the bi-temporal gbrain store.

use rusqlite::ffi::sqlite3_auto_extension;
use rusqlite::{Connection, params};
use std::ffi::{c_char, c_int, c_void};
use std::sync::Once;

pub const SCHEMA_VERSION: i64 = 1;

static SQLITE_VEC_AUTO_EXTENSION: Once = Once::new();

type SqliteVecAutoExtension = unsafe extern "C" fn(
    *mut rusqlite::ffi::sqlite3,
    *mut *mut c_char,
    *const rusqlite::ffi::sqlite3_api_routines,
) -> c_int;

unsafe extern "C" {
    #[link_name = "sqlite3_vec_init"]
    fn sqlite3_vec_init_for_connection(
        db: *mut rusqlite::ffi::sqlite3,
        pz_err_msg: *mut *mut c_char,
        sqlite_api: *const c_void,
    ) -> c_int;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqliteVecStatus {
    pub registered: bool,
    pub available: bool,
    pub error: Option<String>,
}

pub fn register_sqlite_vec(conn: &Connection) -> SqliteVecStatus {
    SQLITE_VEC_AUTO_EXTENSION.call_once(|| unsafe {
        sqlite3_auto_extension(Some(
            std::mem::transmute::<*const (), SqliteVecAutoExtension>(
                sqlite_vec::sqlite3_vec_init as *const (),
            ),
        ));
    });

    unsafe {
        sqlite3_vec_init_for_connection(conn.handle(), std::ptr::null_mut(), std::ptr::null());
    }

    match sqlite_vec_version(conn) {
        Ok(_) => SqliteVecStatus {
            registered: true,
            available: true,
            error: None,
        },
        Err(err) => SqliteVecStatus {
            registered: true,
            available: false,
            error: Some(err.to_string()),
        },
    }
}

pub fn sqlite_vec_available(conn: &Connection) -> bool {
    sqlite_vec_version(conn).is_ok()
}

pub fn sqlite_vec_version(conn: &Connection) -> rusqlite::Result<String> {
    conn.query_row("SELECT vec_version()", [], |row| row.get(0))
}

pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let vec_status = register_sqlite_vec(conn);
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS gbrain_schema (
          version INTEGER NOT NULL PRIMARY KEY,
          applied_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS gbrain_scopes (
          id TEXT PRIMARY KEY,
          kind TEXT NOT NULL,
          value TEXT,
          label TEXT NOT NULL,
          created_at TEXT NOT NULL
        );

        -- Facts are invalidated/retracted, never hard-deleted.
        CREATE TABLE IF NOT EXISTS gbrain_facts (
          id TEXT PRIMARY KEY,
          scope_id TEXT NOT NULL,
          subject TEXT,
          text TEXT NOT NULL,
          content_hash TEXT NOT NULL,
          metadata TEXT NOT NULL DEFAULT '{}',
          valid_at TEXT NOT NULL,
          invalid_at TEXT,
          ingested_at TEXT NOT NULL,
          expired_at TEXT,
          supersedes TEXT,
          superseded_by TEXT,
          supersession_reason TEXT,
          provenance TEXT NOT NULL DEFAULT '[]',
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL,
          FOREIGN KEY(scope_id) REFERENCES gbrain_scopes(id)
        );

        CREATE INDEX IF NOT EXISTS idx_gbrain_facts_scope ON gbrain_facts(scope_id);
        CREATE INDEX IF NOT EXISTS idx_gbrain_facts_subject ON gbrain_facts(scope_id, subject);
        CREATE INDEX IF NOT EXISTS idx_gbrain_facts_valid ON gbrain_facts(valid_at);
        CREATE INDEX IF NOT EXISTS idx_gbrain_facts_ingested ON gbrain_facts(ingested_at);

        CREATE TABLE IF NOT EXISTS gbrain_embeddings (
          fact_id TEXT NOT NULL,
          provider_id TEXT NOT NULL,
          model TEXT NOT NULL,
          dimensions INTEGER NOT NULL,
          embedding BLOB NOT NULL,
          updated_at TEXT NOT NULL,
          PRIMARY KEY(fact_id, provider_id, model),
          FOREIGN KEY(fact_id) REFERENCES gbrain_facts(id)
        );

        -- The supersession / contradiction graph.
        -- kind = 'supersedes' (from_id supersedes to_id) | 'contradicts' (undirected, stored canonically from<to).
        CREATE TABLE IF NOT EXISTS gbrain_links (
          from_id TEXT NOT NULL,
          to_id TEXT NOT NULL,
          kind TEXT NOT NULL,
          reason TEXT,
          created_at TEXT NOT NULL,
          PRIMARY KEY(from_id, to_id, kind),
          FOREIGN KEY(from_id) REFERENCES gbrain_facts(id),
          FOREIGN KEY(to_id) REFERENCES gbrain_facts(id)
        );

        CREATE INDEX IF NOT EXISTS idx_gbrain_links_to ON gbrain_links(to_id, kind);
        CREATE INDEX IF NOT EXISTS idx_gbrain_links_from ON gbrain_links(from_id, kind);

        CREATE TABLE IF NOT EXISTS gbrain_import_runs (
          id TEXT PRIMARY KEY,
          scope_id TEXT NOT NULL,
          source_path TEXT,
          source_hash TEXT,
          started_at TEXT NOT NULL,
          finished_at TEXT,
          status TEXT NOT NULL,
          error TEXT,
          metadata TEXT NOT NULL DEFAULT '{}',
          FOREIGN KEY(scope_id) REFERENCES gbrain_scopes(id)
        );

        CREATE TABLE IF NOT EXISTS gbrain_import_manifest (
          id TEXT PRIMARY KEY,
          import_run_id TEXT,
          source_hash TEXT NOT NULL,
          source_path TEXT,
          source_prefix TEXT,
          corpus_prefix TEXT,
          replacement_policy TEXT,
          fact_count INTEGER NOT NULL DEFAULT 0,
          statement_count INTEGER NOT NULL DEFAULT 0,
          node_count INTEGER NOT NULL DEFAULT 0,
          edge_count INTEGER NOT NULL DEFAULT 0,
          added_at TEXT NOT NULL,
          metadata TEXT NOT NULL DEFAULT '{}',
          FOREIGN KEY(import_run_id) REFERENCES gbrain_import_runs(id)
        );

        CREATE TABLE IF NOT EXISTS gbrain_dream_runs (
          id TEXT PRIMARY KEY,
          scope_id TEXT NOT NULL,
          mode TEXT NOT NULL CHECK(mode IN ('enrich', 'refine', 'compact', 'full')),
          started_at TEXT NOT NULL,
          finished_at TEXT,
          status TEXT NOT NULL CHECK(status IN ('running', 'completed', 'failed', 'canceled')),
          algorithm_version TEXT NOT NULL,
          reasoner_model TEXT,
          run_policy TEXT NOT NULL CHECK(run_policy IN ('interactive', 'eval', 'import', 'maintenance')),
          external_cancellation_token TEXT,
          workers INTEGER NOT NULL DEFAULT 1,
          input_fact_count INTEGER NOT NULL DEFAULT 0,
          derived_statement_count INTEGER NOT NULL DEFAULT 0,
          derived_event_count INTEGER NOT NULL DEFAULT 0,
          invalidated_event_count INTEGER NOT NULL DEFAULT 0,
          error TEXT,
          FOREIGN KEY(scope_id) REFERENCES gbrain_scopes(id)
        );

        CREATE TABLE IF NOT EXISTS gbrain_dream_schedule_leases (
          scope_id TEXT PRIMARY KEY,
          lease_owner TEXT,
          lease_until TEXT,
          last_checked_at TEXT,
          last_scheduled_run_id TEXT,
          updated_at TEXT NOT NULL,
          FOREIGN KEY(scope_id) REFERENCES gbrain_scopes(id),
          FOREIGN KEY(last_scheduled_run_id) REFERENCES gbrain_dream_runs(id)
        );

        CREATE TABLE IF NOT EXISTS gbrain_statements (
          id TEXT PRIMARY KEY,
          scope_id TEXT NOT NULL,
          dream_run_id TEXT,
          source_fact_id TEXT,
          source_slug TEXT,
          text TEXT NOT NULL,
          quote_start INTEGER,
          quote_end INTEGER,
          confidence TEXT NOT NULL CHECK(confidence IN ('EXTRACTED', 'INFERRED', 'AMBIGUOUS')),
          derivation_version TEXT NOT NULL,
          active INTEGER NOT NULL DEFAULT 1,
          created_at TEXT NOT NULL,
          FOREIGN KEY(scope_id) REFERENCES gbrain_scopes(id),
          FOREIGN KEY(dream_run_id) REFERENCES gbrain_dream_runs(id),
          FOREIGN KEY(source_fact_id) REFERENCES gbrain_facts(id)
        );

        CREATE TABLE IF NOT EXISTS gbrain_entities (
          id TEXT PRIMARY KEY,
          scope_id TEXT NOT NULL,
          canonical_name TEXT NOT NULL,
          entity_type TEXT NOT NULL,
          aliases TEXT NOT NULL DEFAULT '[]',
          source_statement_id TEXT,
          dream_run_id TEXT,
          active INTEGER NOT NULL DEFAULT 1,
          created_at TEXT NOT NULL,
          FOREIGN KEY(scope_id) REFERENCES gbrain_scopes(id),
          FOREIGN KEY(source_statement_id) REFERENCES gbrain_statements(id),
          FOREIGN KEY(dream_run_id) REFERENCES gbrain_dream_runs(id)
        );

        CREATE TABLE IF NOT EXISTS gbrain_predicates (
          id TEXT PRIMARY KEY,
          family TEXT NOT NULL,
          label TEXT NOT NULL,
          description TEXT,
          dream_run_id TEXT,
          active INTEGER NOT NULL DEFAULT 1,
          created_at TEXT NOT NULL,
          FOREIGN KEY(dream_run_id) REFERENCES gbrain_dream_runs(id)
        );

        CREATE TABLE IF NOT EXISTS gbrain_ontology_nodes (
          id TEXT PRIMARY KEY,
          version TEXT NOT NULL,
          label TEXT NOT NULL,
          node_class TEXT NOT NULL,
          description TEXT,
          dream_run_id TEXT,
          source_fact_id TEXT,
          active INTEGER NOT NULL DEFAULT 1,
          embedding_key TEXT,
          created_at TEXT NOT NULL,
          FOREIGN KEY(dream_run_id) REFERENCES gbrain_dream_runs(id),
          FOREIGN KEY(source_fact_id) REFERENCES gbrain_facts(id)
        );

        CREATE TABLE IF NOT EXISTS gbrain_ontology_edges (
          id TEXT PRIMARY KEY,
          version TEXT NOT NULL,
          source_ontology_node_id TEXT NOT NULL,
          target_ontology_node_id TEXT NOT NULL,
          relation TEXT NOT NULL,
          rationale TEXT,
          evidence_type TEXT,
          traversal_hint TEXT,
          dream_run_id TEXT,
          active INTEGER NOT NULL DEFAULT 1,
          created_at TEXT NOT NULL,
          FOREIGN KEY(source_ontology_node_id) REFERENCES gbrain_ontology_nodes(id),
          FOREIGN KEY(target_ontology_node_id) REFERENCES gbrain_ontology_nodes(id),
          FOREIGN KEY(dream_run_id) REFERENCES gbrain_dream_runs(id)
        );

        CREATE TABLE IF NOT EXISTS gbrain_temporal_events (
          id TEXT PRIMARY KEY,
          scope_id TEXT NOT NULL,
          dream_run_id TEXT,
          source_statement_id TEXT,
          predicate_id TEXT,
          temporal_class TEXT NOT NULL,
          episodic_class TEXT,
          entity_ids TEXT NOT NULL DEFAULT '[]',
          valid_at TEXT,
          invalid_at TEXT,
          transaction_at TEXT,
          confidence TEXT NOT NULL CHECK(confidence IN ('EXTRACTED', 'INFERRED', 'AMBIGUOUS')),
          source_spans TEXT NOT NULL DEFAULT '[]',
          active INTEGER NOT NULL DEFAULT 1,
          superseded_by TEXT,
          created_at TEXT NOT NULL,
          FOREIGN KEY(scope_id) REFERENCES gbrain_scopes(id),
          FOREIGN KEY(dream_run_id) REFERENCES gbrain_dream_runs(id),
          FOREIGN KEY(source_statement_id) REFERENCES gbrain_statements(id),
          FOREIGN KEY(predicate_id) REFERENCES gbrain_predicates(id)
        );

        CREATE TABLE IF NOT EXISTS gbrain_evidence_cards (
          id TEXT PRIMARY KEY,
          scope_id TEXT NOT NULL,
          dream_run_id TEXT,
          title TEXT NOT NULL,
          summary TEXT NOT NULL,
          quote_spans TEXT NOT NULL DEFAULT '[]',
          source_fact_ids TEXT NOT NULL DEFAULT '[]',
          temporal_status TEXT,
          neighboring_event_ids TEXT NOT NULL DEFAULT '[]',
          confidence TEXT NOT NULL CHECK(confidence IN ('EXTRACTED', 'INFERRED', 'AMBIGUOUS')),
          active INTEGER NOT NULL DEFAULT 1,
          created_at TEXT NOT NULL,
          FOREIGN KEY(scope_id) REFERENCES gbrain_scopes(id),
          FOREIGN KEY(dream_run_id) REFERENCES gbrain_dream_runs(id)
        );

        CREATE TABLE IF NOT EXISTS gbrain_dream_links (
          from_id TEXT NOT NULL,
          to_id TEXT NOT NULL,
          kind TEXT NOT NULL CHECK(kind IN ('mentions', 'supports', 'refines', 'supersedes', 'invalidates', 'contradicts', 'summarizes')),
          dream_run_id TEXT,
          confidence TEXT NOT NULL CHECK(confidence IN ('EXTRACTED', 'INFERRED', 'AMBIGUOUS')),
          evidence_ids TEXT NOT NULL DEFAULT '[]',
          created_at TEXT NOT NULL,
          PRIMARY KEY(from_id, to_id, kind),
          FOREIGN KEY(dream_run_id) REFERENCES gbrain_dream_runs(id)
        );

        CREATE TABLE IF NOT EXISTS gbrain_relevance (
          object_id TEXT NOT NULL,
          object_kind TEXT NOT NULL,
          scope_id TEXT NOT NULL,
          recency_score REAL NOT NULL DEFAULT 0,
          trust_score REAL NOT NULL DEFAULT 0,
          query_frequency INTEGER NOT NULL DEFAULT 0,
          eval_failure_count INTEGER NOT NULL DEFAULT 0,
          retrieval_priority REAL NOT NULL DEFAULT 0,
          dream_run_id TEXT,
          updated_at TEXT NOT NULL,
          PRIMARY KEY(object_id, object_kind, scope_id),
          FOREIGN KEY(scope_id) REFERENCES gbrain_scopes(id),
          FOREIGN KEY(dream_run_id) REFERENCES gbrain_dream_runs(id)
        );

        CREATE TABLE IF NOT EXISTS gbrain_nodes (
          id TEXT PRIMARY KEY,
          label TEXT NOT NULL,
          node_kind TEXT NOT NULL,
          source_artifact TEXT,
          source_location TEXT,
          source_span TEXT,
          source_fact_id TEXT,
          source_statement_id TEXT,
          scope_id TEXT,
          corpus_prefix TEXT,
          created_by_run_id TEXT,
          confidence TEXT NOT NULL CHECK(confidence IN ('EXTRACTED', 'INFERRED', 'AMBIGUOUS')),
          active INTEGER NOT NULL DEFAULT 1,
          superseded_by TEXT,
          embedding_key TEXT,
          created_at TEXT NOT NULL,
          FOREIGN KEY(source_fact_id) REFERENCES gbrain_facts(id),
          FOREIGN KEY(source_statement_id) REFERENCES gbrain_statements(id),
          FOREIGN KEY(scope_id) REFERENCES gbrain_scopes(id),
          FOREIGN KEY(created_by_run_id) REFERENCES gbrain_dream_runs(id)
        );

        CREATE TABLE IF NOT EXISTS gbrain_edges (
          id TEXT PRIMARY KEY,
          source_node_id TEXT NOT NULL,
          target_node_id TEXT NOT NULL,
          relation TEXT NOT NULL,
          confidence TEXT NOT NULL CHECK(confidence IN ('EXTRACTED', 'INFERRED', 'AMBIGUOUS')),
          source_artifact TEXT,
          source_location TEXT,
          source_span TEXT,
          evidence_ids TEXT NOT NULL DEFAULT '[]',
          directed INTEGER NOT NULL DEFAULT 1,
          original_direction TEXT,
          ontology_edge_id TEXT,
          dream_run_id TEXT,
          valid_at TEXT,
          invalid_at TEXT,
          transaction_at TEXT,
          active INTEGER NOT NULL DEFAULT 1,
          superseded_by TEXT,
          created_at TEXT NOT NULL,
          CHECK(source_node_id <> target_node_id),
          FOREIGN KEY(source_node_id) REFERENCES gbrain_nodes(id),
          FOREIGN KEY(target_node_id) REFERENCES gbrain_nodes(id),
          FOREIGN KEY(ontology_edge_id) REFERENCES gbrain_ontology_edges(id),
          FOREIGN KEY(dream_run_id) REFERENCES gbrain_dream_runs(id)
        );

        CREATE TABLE IF NOT EXISTS gbrain_hyperedges (
          id TEXT PRIMARY KEY,
          kind TEXT NOT NULL,
          node_ids TEXT NOT NULL DEFAULT '[]',
          evidence_ids TEXT NOT NULL DEFAULT '[]',
          participants TEXT NOT NULL DEFAULT '[]',
          rationale TEXT,
          resolution TEXT,
          confidence TEXT NOT NULL CHECK(confidence IN ('EXTRACTED', 'INFERRED', 'AMBIGUOUS')),
          dream_run_id TEXT,
          active INTEGER NOT NULL DEFAULT 1,
          created_at TEXT NOT NULL,
          FOREIGN KEY(dream_run_id) REFERENCES gbrain_dream_runs(id)
        );

        CREATE TABLE IF NOT EXISTS gbrain_node_aliases (
          alias_id TEXT PRIMARY KEY,
          canonical_node_id TEXT NOT NULL,
          alias_label TEXT NOT NULL,
          alias_kind TEXT,
          source_artifact TEXT,
          dream_run_id TEXT,
          created_at TEXT NOT NULL,
          FOREIGN KEY(canonical_node_id) REFERENCES gbrain_nodes(id),
          FOREIGN KEY(dream_run_id) REFERENCES gbrain_dream_runs(id)
        );

        CREATE TABLE IF NOT EXISTS gbrain_id_remaps (
          old_id TEXT PRIMARY KEY,
          new_id TEXT NOT NULL,
          reason TEXT,
          dream_run_id TEXT,
          created_at TEXT NOT NULL,
          FOREIGN KEY(new_id) REFERENCES gbrain_nodes(id),
          FOREIGN KEY(dream_run_id) REFERENCES gbrain_dream_runs(id)
        );

        CREATE TABLE IF NOT EXISTS gbrain_communities (
          id TEXT PRIMARY KEY,
          scope_id TEXT NOT NULL,
          version TEXT NOT NULL,
          label TEXT,
          cohesion_score REAL NOT NULL DEFAULT 0,
          hub_filtered INTEGER NOT NULL DEFAULT 0,
          noise_filtered INTEGER NOT NULL DEFAULT 0,
          dream_run_id TEXT,
          active INTEGER NOT NULL DEFAULT 1,
          created_at TEXT NOT NULL,
          FOREIGN KEY(scope_id) REFERENCES gbrain_scopes(id),
          FOREIGN KEY(dream_run_id) REFERENCES gbrain_dream_runs(id)
        );

        CREATE TABLE IF NOT EXISTS gbrain_community_members (
          community_id TEXT NOT NULL,
          node_id TEXT NOT NULL,
          version TEXT NOT NULL,
          membership_score REAL NOT NULL DEFAULT 0,
          hub INTEGER NOT NULL DEFAULT 0,
          noise INTEGER NOT NULL DEFAULT 0,
          dream_run_id TEXT,
          created_at TEXT NOT NULL,
          PRIMARY KEY(community_id, node_id, version),
          FOREIGN KEY(community_id) REFERENCES gbrain_communities(id),
          FOREIGN KEY(node_id) REFERENCES gbrain_nodes(id),
          FOREIGN KEY(dream_run_id) REFERENCES gbrain_dream_runs(id)
        );

        CREATE TABLE IF NOT EXISTS gbrain_query_feedback (
          id TEXT PRIMARY KEY,
          scope_id TEXT,
          question TEXT NOT NULL,
          question_kind TEXT,
          used_nodes TEXT NOT NULL DEFAULT '[]',
          used_cards TEXT NOT NULL DEFAULT '[]',
          used_events TEXT NOT NULL DEFAULT '[]',
          duration_ms INTEGER,
          tool_call_count INTEGER NOT NULL DEFAULT 0,
          stop_reason TEXT,
          answer_length INTEGER,
          response_hash TEXT,
          eval_result_id TEXT,
          created_at TEXT NOT NULL,
          FOREIGN KEY(scope_id) REFERENCES gbrain_scopes(id)
        );

        CREATE INDEX IF NOT EXISTS idx_gbrain_nodes_scope_kind ON gbrain_nodes(scope_id, node_kind);
        CREATE INDEX IF NOT EXISTS idx_gbrain_edges_source ON gbrain_edges(source_node_id, relation);
        CREATE INDEX IF NOT EXISTS idx_gbrain_edges_target ON gbrain_edges(target_node_id, relation);
        CREATE INDEX IF NOT EXISTS idx_gbrain_temporal_events_scope ON gbrain_temporal_events(scope_id, temporal_class);
        CREATE INDEX IF NOT EXISTS idx_gbrain_evidence_cards_scope ON gbrain_evidence_cards(scope_id, active);
        "#,
    )?;
    record_sqlite_vec_status(conn, &vec_status)?;
    if vec_status.available
        && let Err(err) = create_vec0_tables(conn)
    {
        record_sqlite_vec_status(
            conn,
            &SqliteVecStatus {
                registered: true,
                available: false,
                error: Some(format!("vec0 table creation failed: {err}")),
            },
        )?;
    }
    ensure_column(conn, "gbrain_query_feedback", "stop_reason", "TEXT")?;
    conn.execute(
        "INSERT OR IGNORE INTO gbrain_schema(version, applied_at) VALUES (?1, datetime('now'))",
        [SCHEMA_VERSION],
    )?;
    Ok(())
}

fn ensure_column(
    conn: &Connection,
    table: &str,
    column: &str,
    column_type: &str,
) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for existing in columns {
        if existing? == column {
            return Ok(());
        }
    }
    conn.execute(
        &format!("ALTER TABLE {table} ADD COLUMN {column} {column_type}"),
        [],
    )?;
    Ok(())
}

fn record_sqlite_vec_status(conn: &Connection, status: &SqliteVecStatus) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO gbrain_schema(version, applied_at) VALUES (?1, ?2)",
        params![
            -1,
            if status.available {
                "sqlite-vec:available".to_string()
            } else {
                format!(
                    "sqlite-vec:unavailable:{}",
                    status.error.as_deref().unwrap_or("unknown")
                )
            },
        ],
    )?;
    Ok(())
}

fn create_vec0_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        CREATE VIRTUAL TABLE IF NOT EXISTS gbrain_vec_raw_facts USING vec0(
          fact_id TEXT PRIMARY KEY,
          embedding FLOAT[1536],
          provider_id TEXT,
          model TEXT,
          dimensions INTEGER,
          scope_id TEXT
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS gbrain_vec_temporal_events USING vec0(
          event_id TEXT PRIMARY KEY,
          embedding FLOAT[1536],
          provider_id TEXT,
          model TEXT,
          dimensions INTEGER,
          scope_id TEXT
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS gbrain_vec_evidence_cards USING vec0(
          card_id TEXT PRIMARY KEY,
          embedding FLOAT[1536],
          provider_id TEXT,
          model TEXT,
          dimensions INTEGER,
          scope_id TEXT
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS gbrain_vec_ontology_nodes USING vec0(
          ontology_node_id TEXT PRIMARY KEY,
          embedding FLOAT[1536],
          provider_id TEXT,
          model TEXT,
          dimensions INTEGER,
          scope_id TEXT
        );
        "#,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_migrates_gbrain_tables() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        // Idempotent.
        migrate(&conn).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name LIKE 'gbrain_%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(count >= 5, "expected >=5 gbrain_ tables, got {count}");
    }
}
