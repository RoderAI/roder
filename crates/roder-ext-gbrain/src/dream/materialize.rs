use std::collections::HashSet;

use roder_api::memory::MemoryScope;
use rusqlite::{Connection, params};
use serde_json::Value;
use serde_json::json;
use time::OffsetDateTime;

use crate::dream::{GraphIdParts, normalize_graph_id, seed_ontology_edges, seed_ontology_nodes};
use crate::model::format_time;

const CONFIDENCE_EXTRACTED: &str = "EXTRACTED";
const DREAM_GRAPH_VERSION: &str = "phase84-materialized-v2";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GraphMaterializationStats {
    pub fact_nodes: usize,
    pub metadata_nodes: usize,
    pub link_edges: usize,
    pub metadata_edges: usize,
    pub evidence_cards: usize,
    pub ontology_nodes: usize,
    pub ontology_edges: usize,
}

impl GraphMaterializationStats {
    pub fn derived_statement_count(self) -> usize {
        self.fact_nodes + self.metadata_nodes + self.evidence_cards + self.ontology_nodes
    }

    pub fn derived_event_count(self) -> usize {
        self.link_edges + self.metadata_edges + self.ontology_edges
    }
}

pub fn materialize_dream_graph(
    conn: &Connection,
    scope: &MemoryScope,
    dream_run_id: &str,
    now: OffsetDateTime,
) -> anyhow::Result<GraphMaterializationStats> {
    let scope_id = scope.stable_id();
    let now = format_time(now);
    let ontology_nodes = upsert_ontology_nodes(conn, dream_run_id, &now)?;
    let ontology_edges = upsert_ontology_edges(conn, dream_run_id, &now)?;
    let fact_nodes = upsert_fact_nodes(conn, &scope_id, dream_run_id, &now)?;
    let metadata_stats = upsert_metadata_graph(conn, &scope_id, dream_run_id, &now)?;
    let evidence_cards = upsert_evidence_cards(conn, &scope_id, dream_run_id, &now)?;
    let link_edges = upsert_link_edges(conn, &scope_id, dream_run_id, &now)?;

    Ok(GraphMaterializationStats {
        fact_nodes,
        metadata_nodes: metadata_stats.metadata_nodes,
        link_edges,
        metadata_edges: metadata_stats.metadata_edges,
        evidence_cards,
        ontology_nodes,
        ontology_edges,
    })
}

fn upsert_ontology_nodes(
    conn: &Connection,
    dream_run_id: &str,
    now: &str,
) -> anyhow::Result<usize> {
    let mut count = 0;
    for node in seed_ontology_nodes() {
        conn.execute(
            "INSERT INTO gbrain_ontology_nodes(
                id, version, label, node_class, description, dream_run_id, active, created_at
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7)
             ON CONFLICT(id) DO UPDATE SET
                version = excluded.version,
                label = excluded.label,
                node_class = excluded.node_class,
                description = excluded.description,
                dream_run_id = excluded.dream_run_id,
                active = 1",
            params![
                node.id,
                DREAM_GRAPH_VERSION,
                node.label,
                node.node_kind,
                node.explanation,
                dream_run_id,
                now,
            ],
        )?;
        count += 1;
    }
    Ok(count)
}

fn upsert_ontology_edges(
    conn: &Connection,
    dream_run_id: &str,
    now: &str,
) -> anyhow::Result<usize> {
    let mut count = 0;
    for edge in seed_ontology_edges() {
        conn.execute(
            "INSERT INTO gbrain_ontology_edges(
                id, version, source_ontology_node_id, target_ontology_node_id, relation,
                rationale, evidence_type, traversal_hint, dream_run_id, active, created_at
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8, 1, ?9)
             ON CONFLICT(id) DO UPDATE SET
                version = excluded.version,
                source_ontology_node_id = excluded.source_ontology_node_id,
                target_ontology_node_id = excluded.target_ontology_node_id,
                relation = excluded.relation,
                rationale = excluded.rationale,
                evidence_type = excluded.evidence_type,
                dream_run_id = excluded.dream_run_id,
                active = 1",
            params![
                edge.id,
                DREAM_GRAPH_VERSION,
                edge.source_node_id,
                edge.target_node_id,
                edge.relation,
                edge.explanation,
                edge.evidence_type,
                dream_run_id,
                now,
            ],
        )?;
        count += 1;
    }
    Ok(count)
}

fn upsert_fact_nodes(
    conn: &Connection,
    scope_id: &str,
    dream_run_id: &str,
    now: &str,
) -> anyhow::Result<usize> {
    let mut stmt = conn.prepare(
        "SELECT id, subject, text, provenance, expired_at, superseded_by
         FROM gbrain_facts
         WHERE scope_id = ?1
         ORDER BY ingested_at DESC",
    )?;
    let rows = stmt.query_map(params![scope_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, Option<String>>(5)?,
        ))
    })?;

    let mut count = 0;
    for row in rows {
        let (fact_id, subject, text, provenance, expired_at, superseded_by) = row?;
        let source_artifact = source_artifact(&provenance);
        let label = label_for_fact(subject.as_deref(), source_artifact.as_deref(), &text);
        conn.execute(
            "INSERT INTO gbrain_nodes(
                id, label, node_kind, source_artifact, source_location, source_span,
                source_fact_id, source_statement_id, scope_id, corpus_prefix,
                created_by_run_id, confidence, active, superseded_by, embedding_key, created_at
             )
             VALUES (?1, ?2, 'artifact', ?3, ?4, NULL, ?5, NULL, ?6, NULL, ?7, ?8, ?9, ?10, NULL, ?11)
             ON CONFLICT(id) DO UPDATE SET
                label = excluded.label,
                node_kind = excluded.node_kind,
                source_artifact = excluded.source_artifact,
                source_location = excluded.source_location,
                source_fact_id = excluded.source_fact_id,
                scope_id = excluded.scope_id,
                created_by_run_id = excluded.created_by_run_id,
                confidence = excluded.confidence,
                active = excluded.active,
                superseded_by = excluded.superseded_by",
            params![
                fact_node_id(scope_id, &fact_id),
                label,
                source_artifact,
                source_location(&provenance),
                fact_id,
                scope_id,
                dream_run_id,
                CONFIDENCE_EXTRACTED,
                if expired_at.is_none() { 1_i64 } else { 0_i64 },
                superseded_by,
                now,
            ],
        )?;
        count += 1;
    }
    Ok(count)
}

fn upsert_metadata_graph(
    conn: &Connection,
    scope_id: &str,
    dream_run_id: &str,
    now: &str,
) -> anyhow::Result<GraphMaterializationStats> {
    let mut stmt = conn.prepare(
        "SELECT id, metadata, provenance, expired_at, valid_at, invalid_at, ingested_at
         FROM gbrain_facts
         WHERE scope_id = ?1
         ORDER BY ingested_at DESC",
    )?;
    let rows = stmt.query_map(params![scope_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, String>(6)?,
        ))
    })?;

    let mut node_ids = HashSet::new();
    let mut edge_ids = HashSet::new();
    for row in rows {
        let (fact_id, metadata, provenance, expired_at, valid_at, invalid_at, ingested_at) = row?;
        let metadata = serde_json::from_str::<Value>(&metadata).unwrap_or(Value::Null);
        let fact_node_id = fact_node_id(scope_id, &fact_id);
        let source_artifact = source_artifact(&provenance);
        let source_location = source_location(&provenance);
        let active = expired_at.is_none();

        if let Some(thread_id) = metadata_string(&metadata, "thread_id") {
            let node_id = metadata_node_id(scope_id, "thread", &thread_id);
            upsert_metadata_node(
                conn,
                MetadataNodeInput {
                    id: &node_id,
                    label: &thread_id,
                    kind: "thread",
                    scope_id,
                    dream_run_id,
                    now,
                },
            )?;
            node_ids.insert(node_id.clone());
            let edge_id = metadata_edge_id(scope_id, "member_of_thread", &fact_id, &thread_id);
            upsert_metadata_edge(
                conn,
                MetadataEdgeInput {
                    id: &edge_id,
                    source_node_id: &fact_node_id,
                    target_node_id: &node_id,
                    relation: "member_of_thread",
                    source_artifact: source_artifact.as_deref(),
                    source_location: source_location.as_deref(),
                    dream_run_id,
                    valid_at: Some(&valid_at),
                    invalid_at: invalid_at.as_deref(),
                    transaction_at: &ingested_at,
                    active,
                    now,
                },
            )?;
            edge_ids.insert(edge_id);
        }

        if let Some(author) = metadata_string(&metadata, "author") {
            let node_id = metadata_node_id(scope_id, "person", &author);
            upsert_metadata_node(
                conn,
                MetadataNodeInput {
                    id: &node_id,
                    label: &author,
                    kind: "person",
                    scope_id,
                    dream_run_id,
                    now,
                },
            )?;
            node_ids.insert(node_id.clone());
            let edge_id = metadata_edge_id(scope_id, "authored", &author, &fact_id);
            upsert_metadata_edge(
                conn,
                MetadataEdgeInput {
                    id: &edge_id,
                    source_node_id: &node_id,
                    target_node_id: &fact_node_id,
                    relation: "authored",
                    source_artifact: source_artifact.as_deref(),
                    source_location: source_location.as_deref(),
                    dream_run_id,
                    valid_at: Some(&valid_at),
                    invalid_at: invalid_at.as_deref(),
                    transaction_at: &ingested_at,
                    active,
                    now,
                },
            )?;
            edge_ids.insert(edge_id);
        }

        if let Some(source_type) = metadata_string(&metadata, "source_type") {
            let node_id = metadata_node_id(scope_id, "source_type", &source_type);
            upsert_metadata_node(
                conn,
                MetadataNodeInput {
                    id: &node_id,
                    label: &source_type,
                    kind: "source_type",
                    scope_id,
                    dream_run_id,
                    now,
                },
            )?;
            node_ids.insert(node_id.clone());
            let edge_id = metadata_edge_id(scope_id, "has_source_type", &fact_id, &source_type);
            upsert_metadata_edge(
                conn,
                MetadataEdgeInput {
                    id: &edge_id,
                    source_node_id: &fact_node_id,
                    target_node_id: &node_id,
                    relation: "has_source_type",
                    source_artifact: source_artifact.as_deref(),
                    source_location: source_location.as_deref(),
                    dream_run_id,
                    valid_at: Some(&valid_at),
                    invalid_at: invalid_at.as_deref(),
                    transaction_at: &ingested_at,
                    active,
                    now,
                },
            )?;
            edge_ids.insert(edge_id);
        }
    }

    Ok(GraphMaterializationStats {
        metadata_nodes: node_ids.len(),
        metadata_edges: edge_ids.len(),
        ..GraphMaterializationStats::default()
    })
}

struct MetadataNodeInput<'a> {
    id: &'a str,
    label: &'a str,
    kind: &'a str,
    scope_id: &'a str,
    dream_run_id: &'a str,
    now: &'a str,
}

fn upsert_metadata_node(conn: &Connection, input: MetadataNodeInput<'_>) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO gbrain_nodes(
            id, label, node_kind, source_artifact, source_location, source_span,
            source_fact_id, source_statement_id, scope_id, corpus_prefix,
            created_by_run_id, confidence, active, superseded_by, embedding_key, created_at
         )
         VALUES (?1, ?2, ?3, NULL, NULL, NULL, NULL, NULL, ?4, NULL, ?5, ?6, 1, NULL, NULL, ?7)
         ON CONFLICT(id) DO UPDATE SET
            label = excluded.label,
            node_kind = excluded.node_kind,
            scope_id = excluded.scope_id,
            created_by_run_id = excluded.created_by_run_id,
            confidence = excluded.confidence,
            active = 1",
        params![
            input.id,
            input.label,
            input.kind,
            input.scope_id,
            input.dream_run_id,
            CONFIDENCE_EXTRACTED,
            input.now,
        ],
    )?;
    Ok(())
}

struct MetadataEdgeInput<'a> {
    id: &'a str,
    source_node_id: &'a str,
    target_node_id: &'a str,
    relation: &'a str,
    source_artifact: Option<&'a str>,
    source_location: Option<&'a str>,
    dream_run_id: &'a str,
    valid_at: Option<&'a str>,
    invalid_at: Option<&'a str>,
    transaction_at: &'a str,
    active: bool,
    now: &'a str,
}

fn upsert_metadata_edge(conn: &Connection, input: MetadataEdgeInput<'_>) -> anyhow::Result<()> {
    if input.source_node_id == input.target_node_id {
        return Ok(());
    }
    conn.execute(
        "INSERT INTO gbrain_edges(
            id, source_node_id, target_node_id, relation, confidence, source_artifact,
            source_location, source_span, evidence_ids, directed, original_direction,
            ontology_edge_id, dream_run_id, valid_at, invalid_at, transaction_at,
            active, superseded_by, created_at
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8, 1, 'source_to_target',
                 NULL, ?9, ?10, ?11, ?12, ?13, NULL, ?14)
         ON CONFLICT(id) DO UPDATE SET
            source_node_id = excluded.source_node_id,
            target_node_id = excluded.target_node_id,
            relation = excluded.relation,
            confidence = excluded.confidence,
            source_artifact = excluded.source_artifact,
            source_location = excluded.source_location,
            evidence_ids = excluded.evidence_ids,
            directed = 1,
            original_direction = excluded.original_direction,
            dream_run_id = excluded.dream_run_id,
            valid_at = excluded.valid_at,
            invalid_at = excluded.invalid_at,
            transaction_at = excluded.transaction_at,
            active = excluded.active",
        params![
            input.id,
            input.source_node_id,
            input.target_node_id,
            input.relation,
            CONFIDENCE_EXTRACTED,
            input.source_artifact,
            input.source_location,
            json!([input.source_artifact]).to_string(),
            input.dream_run_id,
            input.valid_at,
            input.invalid_at,
            input.transaction_at,
            if input.active { 1_i64 } else { 0_i64 },
            input.now,
        ],
    )?;
    Ok(())
}

fn upsert_evidence_cards(
    conn: &Connection,
    scope_id: &str,
    dream_run_id: &str,
    now: &str,
) -> anyhow::Result<usize> {
    let mut stmt = conn.prepare(
        "SELECT id, subject, text, provenance, expired_at
         FROM gbrain_facts
         WHERE scope_id = ?1
         ORDER BY ingested_at DESC",
    )?;
    let rows = stmt.query_map(params![scope_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<String>>(4)?,
        ))
    })?;

    let mut count = 0;
    for row in rows {
        let (fact_id, subject, text, provenance, expired_at) = row?;
        let source_artifact = source_artifact(&provenance);
        let title = label_for_fact(subject.as_deref(), source_artifact.as_deref(), &text);
        let active = expired_at.is_none();
        conn.execute(
            "INSERT INTO gbrain_evidence_cards(
                id, scope_id, dream_run_id, title, summary, quote_spans,
                source_fact_ids, temporal_status, neighboring_event_ids,
                confidence, active, created_at
             )
             VALUES (?1, ?2, ?3, ?4, ?5, '[]', ?6, ?7, '[]', ?8, ?9, ?10)
             ON CONFLICT(id) DO UPDATE SET
                scope_id = excluded.scope_id,
                dream_run_id = excluded.dream_run_id,
                title = excluded.title,
                summary = excluded.summary,
                source_fact_ids = excluded.source_fact_ids,
                temporal_status = excluded.temporal_status,
                confidence = excluded.confidence,
                active = excluded.active",
            params![
                evidence_card_id(scope_id, &fact_id),
                scope_id,
                dream_run_id,
                title,
                truncate(&text, 320),
                json!([fact_id]).to_string(),
                if active { "active" } else { "inactive" },
                CONFIDENCE_EXTRACTED,
                if active { 1_i64 } else { 0_i64 },
                now,
            ],
        )?;
        count += 1;
    }
    Ok(count)
}

fn upsert_link_edges(
    conn: &Connection,
    scope_id: &str,
    dream_run_id: &str,
    now: &str,
) -> anyhow::Result<usize> {
    let mut stmt = conn.prepare(
        "SELECT l.from_id, l.to_id, l.kind, l.reason, f.provenance, f.valid_at,
                f.invalid_at, f.ingested_at
         FROM gbrain_links l
         JOIN gbrain_facts f ON f.id = l.from_id
         JOIN gbrain_facts t ON t.id = l.to_id
         WHERE f.scope_id = ?1 AND t.scope_id = ?1
         ORDER BY l.created_at DESC",
    )?;
    let rows = stmt.query_map(params![scope_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, Option<String>>(6)?,
            row.get::<_, String>(7)?,
        ))
    })?;

    let mut count = 0;
    for row in rows {
        let (from_id, to_id, relation, reason, provenance, valid_at, invalid_at, ingested_at) =
            row?;
        let source_node_id = fact_node_id(scope_id, &from_id);
        let target_node_id = fact_node_id(scope_id, &to_id);
        if source_node_id == target_node_id {
            continue;
        }
        conn.execute(
            "INSERT INTO gbrain_edges(
                id, source_node_id, target_node_id, relation, confidence, source_artifact,
                source_location, source_span, evidence_ids, directed, original_direction,
                ontology_edge_id, dream_run_id, valid_at, invalid_at, transaction_at,
                active, superseded_by, created_at
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL, ?12, ?13, ?14, ?15, 1, NULL, ?16)
             ON CONFLICT(id) DO UPDATE SET
                source_node_id = excluded.source_node_id,
                target_node_id = excluded.target_node_id,
                relation = excluded.relation,
                confidence = excluded.confidence,
                source_artifact = excluded.source_artifact,
                source_location = excluded.source_location,
                source_span = excluded.source_span,
                evidence_ids = excluded.evidence_ids,
                directed = excluded.directed,
                original_direction = excluded.original_direction,
                dream_run_id = excluded.dream_run_id,
                valid_at = excluded.valid_at,
                invalid_at = excluded.invalid_at,
                transaction_at = excluded.transaction_at,
                active = 1",
            params![
                link_edge_id(scope_id, &relation, &from_id, &to_id),
                source_node_id,
                target_node_id,
                relation,
                CONFIDENCE_EXTRACTED,
                source_artifact(&provenance),
                source_location(&provenance),
                reason,
                json!([from_id, to_id]).to_string(),
                if relation == "contradicts" { 0_i64 } else { 1_i64 },
                if relation == "contradicts" {
                    "undirected"
                } else {
                    "source_to_target"
                },
                dream_run_id,
                valid_at,
                invalid_at,
                ingested_at,
                now,
            ],
        )?;
        count += 1;
    }
    Ok(count)
}

fn fact_node_id(scope_id: &str, fact_id: &str) -> String {
    normalize_graph_id(GraphIdParts {
        scope: Some(scope_id),
        kind: "artifact",
        label: fact_id,
    })
}

fn evidence_card_id(scope_id: &str, fact_id: &str) -> String {
    normalize_graph_id(GraphIdParts {
        scope: Some(scope_id),
        kind: "evidence",
        label: fact_id,
    })
}

fn metadata_node_id(scope_id: &str, kind: &str, label: &str) -> String {
    normalize_graph_id(GraphIdParts {
        scope: Some(scope_id),
        kind,
        label,
    })
}

fn link_edge_id(scope_id: &str, relation: &str, from_id: &str, to_id: &str) -> String {
    normalize_graph_id(GraphIdParts {
        scope: Some(scope_id),
        kind: "edge",
        label: &format!("{relation}:{from_id}:{to_id}"),
    })
}

fn metadata_edge_id(scope_id: &str, relation: &str, source: &str, target: &str) -> String {
    normalize_graph_id(GraphIdParts {
        scope: Some(scope_id),
        kind: "edge",
        label: &format!("{relation}:{source}:{target}"),
    })
}

fn metadata_string(metadata: &Value, key: &str) -> Option<String> {
    metadata
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn source_artifact(provenance: &str) -> Option<String> {
    serde_json::from_str::<Vec<String>>(provenance)
        .ok()
        .and_then(|items| {
            items
                .into_iter()
                .map(|item| item.trim().to_string())
                .find(|item| !item.is_empty())
        })
}

fn source_location(provenance: &str) -> Option<String> {
    source_artifact(provenance).map(|source| format!("artifact://{source}"))
}

fn label_for_fact(subject: Option<&str>, source_artifact: Option<&str>, text: &str) -> String {
    subject
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or(source_artifact
            .map(str::trim)
            .filter(|value| !value.is_empty()))
        .map(str::to_string)
        .unwrap_or_else(|| truncate(text, 90))
}

fn truncate(value: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for ch in value.trim().chars().take(max_chars) {
        out.push(ch);
    }
    if value.trim().chars().count() > max_chars {
        out.push_str("...");
    }
    out
}

#[cfg(test)]
mod tests {
    use roder_api::memory::MemoryScope;
    use rusqlite::{Connection, params};
    use time::OffsetDateTime;

    use super::materialize_dream_graph;
    use crate::embed::Embedder;
    use crate::model::{content_hash, format_time};
    use crate::schema;
    use crate::store::{GbrainStore, ensure_scope};

    #[test]
    fn dream_materializes_obsidian_graph_rows() {
        let conn = Connection::open_in_memory().unwrap();
        schema::migrate(&conn).unwrap();
        let scope = MemoryScope::Project("helix-small".to_string());
        let scope_id = ensure_scope(&conn, &scope).unwrap();
        let now = OffsetDateTime::now_utc();
        let now_s = format_time(now);

        insert_fact(
            &conn,
            &scope_id,
            "fact-a",
            "Dan owns the launch plan.",
            r#"["doc-a"]"#,
            r#"{"source_type":"slack_thread","author":"Dan","thread_id":"launch"}"#,
            &now_s,
        );
        insert_fact(
            &conn,
            &scope_id,
            "fact-b",
            "Daniel replaced Dan on launch plan ownership.",
            r#"["doc-b"]"#,
            r#"{"source_type":"meeting_notes","author":"Daniel","thread_id":"launch"}"#,
            &now_s,
        );
        conn.execute(
            "INSERT INTO gbrain_links(from_id, to_id, kind, reason, created_at)
             VALUES ('fact-b', 'fact-a', 'supersedes', 'newer assignment', ?1)",
            params![now_s],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO gbrain_dream_runs(
                id, scope_id, mode, started_at, status, algorithm_version, run_policy, workers
             )
             VALUES ('dream-1', ?1, 'refine', ?2, 'running', 'test', 'eval', 1)",
            params![scope_id, now_s],
        )
        .unwrap();

        let stats = materialize_dream_graph(&conn, &scope, "dream-1", now).unwrap();

        assert_eq!(stats.fact_nodes, 2);
        assert_eq!(stats.metadata_nodes, 5);
        assert_eq!(stats.evidence_cards, 2);
        assert_eq!(stats.link_edges, 1);
        assert_eq!(stats.metadata_edges, 6);
        assert!(stats.ontology_nodes > 0);
        assert!(stats.ontology_edges > 0);
        assert_eq!(count(&conn, "gbrain_nodes"), 7);
        assert_eq!(count(&conn, "gbrain_edges"), 7);
        assert_eq!(count(&conn, "gbrain_evidence_cards"), 2);

        let _ = GbrainStore::open_in_memory(Embedder::new(None)).unwrap();
    }

    fn insert_fact(
        conn: &Connection,
        scope_id: &str,
        id: &str,
        text: &str,
        provenance: &str,
        metadata: &str,
        now: &str,
    ) {
        conn.execute(
            "INSERT INTO gbrain_facts(
                id, scope_id, subject, text, content_hash, metadata, valid_at, ingested_at,
                provenance, created_at, updated_at
             )
             VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6, ?6, ?7, ?6, ?6)",
            params![
                id,
                scope_id,
                text,
                content_hash(text),
                metadata,
                now,
                provenance
            ],
        )
        .unwrap();
    }

    fn count(conn: &Connection, table: &str) -> i64 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
    }
}
