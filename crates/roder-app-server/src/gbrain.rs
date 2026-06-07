use std::collections::HashSet;
use std::path::PathBuf;

use roder_protocol::{
    GbrainDreamRunSummary, GbrainEvidenceCard, GbrainGraphEdge, GbrainGraphNode,
    GbrainGraphNodeResult, GbrainGraphParams, GbrainGraphResult, GbrainGraphStats, GbrainOntology,
    GbrainOntologyEdge, GbrainOntologyNode, GbrainSearchParams, GbrainStatusParams,
    GbrainStatusResult, JsonRpcError,
};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde_json::Value;
use time::OffsetDateTime;

use crate::server::AppServer;

const GBRAIN_STORE_ID: &str = "gbrain-bitemporal";
const DEFAULT_LIMIT: usize = 250;
const MAX_LIMIT: usize = 2_000;

impl AppServer {
    pub(crate) async fn handle_gbrain_graph(
        &self,
        params: GbrainGraphParams,
    ) -> Result<Value, JsonRpcError> {
        let store_path = self.gbrain_store_path()?;
        let conn = open_read_only(&store_path)?;
        let result = load_graph(&conn, params).map_err(internal_error)?;
        Ok(serde_json::to_value(result).unwrap())
    }

    pub(crate) async fn handle_gbrain_node(
        &self,
        params: roder_protocol::GbrainNodeParams,
    ) -> Result<Value, JsonRpcError> {
        let store_path = self.gbrain_store_path()?;
        let conn = open_read_only(&store_path)?;
        let graph_params = GbrainGraphParams {
            scope: params.scope,
            query: Some(params.node_id.clone()),
            as_of: None,
            limit: Some(DEFAULT_LIMIT),
            node_kinds: None,
            include_inactive: params.include_inactive,
            include_evidence: params.include_evidence,
        };
        let graph = load_graph(&conn, graph_params).map_err(internal_error)?;
        let node = graph
            .nodes
            .iter()
            .find(|node| node.id == params.node_id)
            .cloned();
        Ok(serde_json::to_value(GbrainGraphNodeResult { graph, node }).unwrap())
    }

    pub(crate) async fn handle_gbrain_search(
        &self,
        params: GbrainSearchParams,
    ) -> Result<Value, JsonRpcError> {
        let graph = GbrainGraphParams {
            scope: params.scope,
            query: Some(params.query),
            as_of: params.as_of,
            limit: params.limit,
            node_kinds: params.node_kinds,
            include_inactive: params.include_inactive,
            include_evidence: params.include_evidence,
        };
        self.handle_gbrain_graph(graph).await
    }

    pub(crate) async fn handle_gbrain_status(
        &self,
        params: GbrainStatusParams,
    ) -> Result<Value, JsonRpcError> {
        let Some(store_path) = self.optional_gbrain_store_path() else {
            let scope = scope_filter(params.scope.as_deref()).map_err(invalid_params)?;
            return Ok(serde_json::to_value(GbrainStatusResult {
                scope_id: scope.display_id(),
                generated_at: now_string(),
                store_path: String::new(),
                available: false,
                latest_dream_run: None,
                stats: GbrainGraphStats::default(),
            })
            .unwrap());
        };
        if !store_path.exists() {
            let scope = scope_filter(params.scope.as_deref()).map_err(invalid_params)?;
            return Ok(serde_json::to_value(GbrainStatusResult {
                scope_id: scope.display_id(),
                generated_at: now_string(),
                store_path: store_path.display().to_string(),
                available: false,
                latest_dream_run: None,
                stats: GbrainGraphStats::default(),
            })
            .unwrap());
        }

        let conn = open_read_only(&store_path)?;
        let scope = scope_filter(params.scope.as_deref()).map_err(invalid_params)?;
        let stats = load_stats(&conn, &scope).map_err(internal_error)?;
        let latest_dream_run = load_dream_runs(&conn, &scope, 1)
            .map_err(internal_error)?
            .into_iter()
            .next();
        Ok(serde_json::to_value(GbrainStatusResult {
            scope_id: scope.display_id(),
            generated_at: now_string(),
            store_path: store_path.display().to_string(),
            available: true,
            latest_dream_run,
            stats,
        })
        .unwrap())
    }

    fn optional_gbrain_store_path(&self) -> Option<PathBuf> {
        self.runtime
            .registry
            .memory_stores
            .iter()
            .find(|factory| factory.id() == GBRAIN_STORE_ID)
            .and_then(|factory| factory.storage_path())
    }

    fn gbrain_store_path(&self) -> Result<PathBuf, JsonRpcError> {
        self.optional_gbrain_store_path().ok_or_else(|| {
            invalid_params(format!(
                "memory store {GBRAIN_STORE_ID:?} is not registered in this Roder runtime"
            ))
        })
    }
}

#[derive(Debug, Clone)]
enum ScopeFilter {
    All,
    One(String),
}

impl ScopeFilter {
    fn display_id(&self) -> String {
        match self {
            Self::All => "all".to_string(),
            Self::One(scope) => scope.clone(),
        }
    }

    fn matches(&self, scope_id: Option<&str>) -> bool {
        match self {
            Self::All => true,
            Self::One(expected) => scope_id == Some(expected.as_str()),
        }
    }
}

fn load_graph(conn: &Connection, params: GbrainGraphParams) -> anyhow::Result<GbrainGraphResult> {
    let scope = scope_filter(params.scope.as_deref())?;
    let limit = params.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let query = params
        .query
        .as_deref()
        .map(str::trim)
        .filter(|query| !query.is_empty())
        .map(str::to_string);
    let as_of = params
        .as_of
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let requested_kinds = params
        .node_kinds
        .unwrap_or_default()
        .into_iter()
        .map(|kind| kind.trim().to_ascii_lowercase())
        .filter(|kind| !kind.is_empty())
        .collect::<HashSet<_>>();

    let mut nodes = load_derived_nodes(
        conn,
        &scope,
        query.as_deref(),
        &requested_kinds,
        params.include_inactive,
        limit,
    )?;
    let fallback_raw = nodes.is_empty();
    if fallback_raw {
        nodes = load_raw_fact_nodes(
            conn,
            &scope,
            query.as_deref(),
            as_of.as_deref(),
            params.include_inactive,
            limit,
        )?;
    }

    let node_ids = nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    let edges = if fallback_raw {
        load_raw_fact_edges(conn, &node_ids, params.include_inactive)?
    } else {
        load_derived_edges(conn, &node_ids, as_of.as_deref(), params.include_inactive)?
    };
    let evidence_cards = if params.include_evidence {
        load_evidence_cards(conn, &scope, params.include_inactive, 200)?
    } else {
        Vec::new()
    };
    let ontology = load_ontology(conn, params.include_inactive)?;
    let dream_runs = load_dream_runs(conn, &scope, 20)?;
    let mut stats = load_stats(conn, &scope)?;
    stats.node_count = nodes.len();
    stats.edge_count = edges.len();
    stats.evidence_card_count = evidence_cards.len();
    stats.ontology_node_count = ontology.nodes.len();
    stats.ontology_edge_count = ontology.edges.len();
    stats.dream_run_count = dream_runs.len();
    stats.fallback_raw = fallback_raw;

    Ok(GbrainGraphResult {
        scope_id: scope.display_id(),
        generated_at: now_string(),
        nodes,
        edges,
        evidence_cards,
        ontology,
        dream_runs,
        stats,
    })
}

fn load_derived_nodes(
    conn: &Connection,
    scope: &ScopeFilter,
    query: Option<&str>,
    requested_kinds: &HashSet<String>,
    include_inactive: bool,
    limit: usize,
) -> anyhow::Result<Vec<GbrainGraphNode>> {
    let mut stmt = conn.prepare(
        "SELECT id, label, node_kind, confidence, active, source_artifact,
                source_fact_id, source_statement_id, created_by_run_id, scope_id
         FROM gbrain_nodes
         ORDER BY active DESC, created_at DESC
         LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit as i64], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, i64>(4)? != 0,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, Option<String>>(6)?,
            row.get::<_, Option<String>>(7)?,
            row.get::<_, Option<String>>(8)?,
            row.get::<_, Option<String>>(9)?,
        ))
    })?;

    let mut out = Vec::new();
    for row in rows {
        let (
            id,
            label,
            kind,
            confidence,
            active,
            source_artifact,
            source_fact_id,
            source_statement_id,
            created_by_run_id,
            scope_id,
        ) = row?;
        if !include_inactive && !active {
            continue;
        }
        if !scope.matches(scope_id.as_deref()) {
            continue;
        }
        if !requested_kinds.is_empty() && !requested_kinds.contains(&kind.to_ascii_lowercase()) {
            continue;
        }
        if !matches_query(
            query,
            [&id, &label, &kind, source_artifact.as_deref().unwrap_or("")],
        ) {
            continue;
        }
        out.push(GbrainGraphNode {
            id,
            label,
            kind,
            confidence,
            active,
            source_artifact,
            source_fact_id,
            source_statement_id,
            created_by_run_id,
            temporal_status: None,
            summary: None,
        });
        if out.len() >= limit {
            break;
        }
    }
    Ok(out)
}

fn load_raw_fact_nodes(
    conn: &Connection,
    scope: &ScopeFilter,
    query: Option<&str>,
    as_of: Option<&str>,
    include_inactive: bool,
    limit: usize,
) -> anyhow::Result<Vec<GbrainGraphNode>> {
    let mut stmt = conn.prepare(
        "SELECT id, scope_id, subject, text, valid_at, invalid_at, expired_at, provenance
         FROM gbrain_facts
         ORDER BY ingested_at DESC
         LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![(limit * 4).min(MAX_LIMIT) as i64], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, Option<String>>(6)?,
            row.get::<_, String>(7)?,
        ))
    })?;

    let mut out = Vec::new();
    for row in rows {
        let (id, scope_id, subject, text, valid_at, invalid_at, expired_at, provenance) = row?;
        if !scope.matches(Some(&scope_id)) {
            continue;
        }
        let active = fact_active(
            as_of,
            &valid_at,
            invalid_at.as_deref(),
            expired_at.as_deref(),
        );
        if !include_inactive && !active {
            continue;
        }
        if !matches_query(query, [&id, subject.as_deref().unwrap_or(""), &text]) {
            continue;
        }
        let source_artifact = json_array_strings(&provenance).into_iter().next();
        out.push(GbrainGraphNode {
            id: id.clone(),
            label: subject
                .filter(|subject| !subject.trim().is_empty())
                .unwrap_or_else(|| truncate(&text, 90)),
            kind: "raw_fact".to_string(),
            confidence: "EXTRACTED".to_string(),
            active,
            source_artifact,
            source_fact_id: Some(id),
            source_statement_id: None,
            created_by_run_id: None,
            temporal_status: Some(if active { "active" } else { "inactive" }.to_string()),
            summary: Some(text),
        });
        if out.len() >= limit {
            break;
        }
    }
    Ok(out)
}

fn load_derived_edges(
    conn: &Connection,
    node_ids: &HashSet<&str>,
    as_of: Option<&str>,
    include_inactive: bool,
) -> anyhow::Result<Vec<GbrainGraphEdge>> {
    if node_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare(
        "SELECT id, source_node_id, target_node_id, relation, confidence, directed,
                valid_at, invalid_at, evidence_ids, active
         FROM gbrain_edges
         ORDER BY active DESC, created_at DESC
         LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![MAX_LIMIT as i64], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, i64>(5)? != 0,
            row.get::<_, Option<String>>(6)?,
            row.get::<_, Option<String>>(7)?,
            row.get::<_, String>(8)?,
            row.get::<_, i64>(9)? != 0,
        ))
    })?;

    let mut out = Vec::new();
    for row in rows {
        let (
            id,
            source,
            target,
            relation,
            confidence,
            directed,
            valid_at,
            invalid_at,
            evidence_ids,
            active,
        ) = row?;
        if !node_ids.contains(source.as_str()) || !node_ids.contains(target.as_str()) {
            continue;
        }
        if !include_inactive && !active {
            continue;
        }
        if !edge_active(as_of, valid_at.as_deref(), invalid_at.as_deref()) {
            continue;
        }
        out.push(GbrainGraphEdge {
            id,
            source,
            target,
            relation,
            confidence,
            directed,
            valid_at,
            invalid_at,
            evidence_ids: json_array_strings(&evidence_ids),
            active,
        });
    }
    Ok(out)
}

fn load_raw_fact_edges(
    conn: &Connection,
    node_ids: &HashSet<&str>,
    _include_inactive: bool,
) -> anyhow::Result<Vec<GbrainGraphEdge>> {
    if node_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare(
        "SELECT from_id, to_id, kind
         FROM gbrain_links
         ORDER BY created_at DESC
         LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![MAX_LIMIT as i64], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;

    let mut out = Vec::new();
    for row in rows {
        let (source, target, relation) = row?;
        if !node_ids.contains(source.as_str()) || !node_ids.contains(target.as_str()) {
            continue;
        }
        out.push(GbrainGraphEdge {
            id: format!("raw-link:{relation}:{source}:{target}"),
            source,
            target,
            relation,
            confidence: "EXTRACTED".to_string(),
            directed: true,
            valid_at: None,
            invalid_at: None,
            evidence_ids: Vec::new(),
            active: true,
        });
    }
    Ok(out)
}

fn load_evidence_cards(
    conn: &Connection,
    scope: &ScopeFilter,
    include_inactive: bool,
    limit: usize,
) -> anyhow::Result<Vec<GbrainEvidenceCard>> {
    let mut stmt = conn.prepare(
        "SELECT id, scope_id, title, summary, quote_spans, source_fact_ids,
                temporal_status, neighboring_event_ids, confidence, active
         FROM gbrain_evidence_cards
         ORDER BY active DESC, created_at DESC
         LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit as i64], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, Option<String>>(6)?,
            row.get::<_, String>(7)?,
            row.get::<_, String>(8)?,
            row.get::<_, i64>(9)? != 0,
        ))
    })?;

    let mut out = Vec::new();
    for row in rows {
        let (
            id,
            scope_id,
            title,
            summary,
            quote_spans,
            source_fact_ids,
            temporal_status,
            neighboring_event_ids,
            confidence,
            active,
        ) = row?;
        if !scope.matches(Some(&scope_id)) || (!include_inactive && !active) {
            continue;
        }
        out.push(GbrainEvidenceCard {
            id,
            title,
            summary,
            quote_spans: serde_json::from_str(&quote_spans).unwrap_or_default(),
            source_fact_ids: json_array_strings(&source_fact_ids),
            temporal_status,
            neighboring_event_ids: json_array_strings(&neighboring_event_ids),
            confidence,
            active,
        });
    }
    Ok(out)
}

fn load_ontology(conn: &Connection, include_inactive: bool) -> anyhow::Result<GbrainOntology> {
    let mut node_stmt = conn.prepare(
        "SELECT id, version, label, node_class, description, active
         FROM gbrain_ontology_nodes
         ORDER BY active DESC, created_at DESC
         LIMIT ?1",
    )?;
    let nodes = node_stmt
        .query_map(params![500_i64], |row| {
            Ok(GbrainOntologyNode {
                id: row.get(0)?,
                version: row.get(1)?,
                label: row.get(2)?,
                node_class: row.get(3)?,
                description: row.get(4)?,
                active: row.get::<_, i64>(5)? != 0,
            })
        })?
        .filter_map(Result::ok)
        .filter(|node| include_inactive || node.active)
        .collect();

    let mut edge_stmt = conn.prepare(
        "SELECT id, version, source_ontology_node_id, target_ontology_node_id,
                relation, rationale, evidence_type, traversal_hint, active
         FROM gbrain_ontology_edges
         ORDER BY active DESC, created_at DESC
         LIMIT ?1",
    )?;
    let edges = edge_stmt
        .query_map(params![500_i64], |row| {
            Ok(GbrainOntologyEdge {
                id: row.get(0)?,
                version: row.get(1)?,
                source: row.get(2)?,
                target: row.get(3)?,
                relation: row.get(4)?,
                rationale: row.get(5)?,
                evidence_type: row.get(6)?,
                traversal_hint: row.get(7)?,
                active: row.get::<_, i64>(8)? != 0,
            })
        })?
        .filter_map(Result::ok)
        .filter(|edge| include_inactive || edge.active)
        .collect();

    Ok(GbrainOntology { nodes, edges })
}

fn load_dream_runs(
    conn: &Connection,
    scope: &ScopeFilter,
    limit: usize,
) -> anyhow::Result<Vec<GbrainDreamRunSummary>> {
    let mut stmt = conn.prepare(
        "SELECT id, scope_id, mode, status, started_at, finished_at, run_policy,
                input_fact_count, derived_event_count
         FROM gbrain_dream_runs
         ORDER BY started_at DESC
         LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit as i64], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, i64>(7)?,
            row.get::<_, i64>(8)?,
        ))
    })?;

    let mut out = Vec::new();
    for row in rows {
        let (
            id,
            scope_id,
            mode,
            status,
            started_at,
            finished_at,
            run_policy,
            input_fact_count,
            derived_event_count,
        ) = row?;
        if !scope.matches(Some(&scope_id)) {
            continue;
        }
        out.push(GbrainDreamRunSummary {
            id,
            mode,
            status,
            started_at,
            finished_at,
            run_policy,
            input_fact_count: input_fact_count.max(0) as usize,
            derived_event_count: derived_event_count.max(0) as usize,
        });
    }
    Ok(out)
}

fn load_stats(conn: &Connection, scope: &ScopeFilter) -> anyhow::Result<GbrainGraphStats> {
    Ok(GbrainGraphStats {
        raw_fact_count: count_scoped(conn, "gbrain_facts", "scope_id", scope)?,
        node_count: count_scoped(conn, "gbrain_nodes", "scope_id", scope)?,
        edge_count: count_table(conn, "gbrain_edges")?,
        evidence_card_count: count_scoped(conn, "gbrain_evidence_cards", "scope_id", scope)?,
        ontology_node_count: count_table(conn, "gbrain_ontology_nodes")?,
        ontology_edge_count: count_table(conn, "gbrain_ontology_edges")?,
        dream_run_count: count_scoped(conn, "gbrain_dream_runs", "scope_id", scope)?,
        fallback_raw: false,
    })
}

fn count_table(conn: &Connection, table: &str) -> anyhow::Result<usize> {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    Ok(conn.query_row(&sql, [], |row| row.get::<_, i64>(0))?.max(0) as usize)
}

fn count_scoped(
    conn: &Connection,
    table: &str,
    scope_column: &str,
    scope: &ScopeFilter,
) -> anyhow::Result<usize> {
    match scope {
        ScopeFilter::All => count_table(conn, table),
        ScopeFilter::One(scope_id) => {
            let sql = format!("SELECT COUNT(*) FROM {table} WHERE {scope_column} = ?1");
            Ok(conn
                .query_row(&sql, params![scope_id], |row| row.get::<_, i64>(0))
                .optional()?
                .unwrap_or_default()
                .max(0) as usize)
        }
    }
}

fn open_read_only(path: &PathBuf) -> Result<Connection, JsonRpcError> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|err| {
        internal_error(format!(
            "open gbrain database {} read-only: {err}",
            path.display()
        ))
    })
}

fn scope_filter(scope: Option<&str>) -> anyhow::Result<ScopeFilter> {
    let Some(scope) = scope.map(str::trim).filter(|scope| !scope.is_empty()) else {
        return Ok(ScopeFilter::One("global".to_string()));
    };
    if scope.eq_ignore_ascii_case("all") {
        return Ok(ScopeFilter::All);
    }
    if scope == "global" {
        return Ok(ScopeFilter::One("global".to_string()));
    }
    for prefix in ["user", "workspace", "project", "thread"] {
        if let Some(value) = scope.strip_prefix(&format!("{prefix}:"))
            && !value.trim().is_empty()
        {
            return Ok(ScopeFilter::One(format!("{prefix}:{}", value.trim())));
        }
    }
    anyhow::bail!(
        "invalid gbrain scope {scope:?}; expected global, all, user:<id>, workspace:<id>, project:<id>, or thread:<id>"
    )
}

fn matches_query<'a>(query: Option<&str>, fields: impl IntoIterator<Item = &'a str>) -> bool {
    let Some(query) = query.map(|value| value.to_ascii_lowercase()) else {
        return true;
    };
    fields
        .into_iter()
        .any(|field| field.to_ascii_lowercase().contains(&query))
}

fn fact_active(
    as_of: Option<&str>,
    valid_at: &str,
    invalid_at: Option<&str>,
    expired_at: Option<&str>,
) -> bool {
    let Some(as_of) = as_of else {
        return expired_at.is_none();
    };
    valid_at <= as_of
        && invalid_at.is_none_or(|invalid_at| invalid_at > as_of)
        && expired_at.is_none_or(|expired_at| expired_at > as_of)
}

fn edge_active(as_of: Option<&str>, valid_at: Option<&str>, invalid_at: Option<&str>) -> bool {
    let Some(as_of) = as_of else {
        return true;
    };
    valid_at.is_none_or(|valid_at| valid_at <= as_of)
        && invalid_at.is_none_or(|invalid_at| invalid_at > as_of)
}

fn json_array_strings(raw: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(raw).unwrap_or_default()
}

fn truncate(text: &str, max_chars: usize) -> String {
    let mut out = text.chars().take(max_chars).collect::<String>();
    if text.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}

fn now_string() -> String {
    OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn invalid_params(err: impl std::fmt::Display) -> JsonRpcError {
    JsonRpcError {
        code: -32602,
        message: err.to_string(),
        data: None,
    }
}

fn internal_error(err: impl std::fmt::Display) -> JsonRpcError {
    JsonRpcError {
        code: -32000,
        message: err.to_string(),
        data: None,
    }
}
