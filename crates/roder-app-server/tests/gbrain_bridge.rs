use std::sync::Arc;

use roder_api::extension::ExtensionRegistryBuilder;
use roder_api::memory::MemoryScope;
use roder_app_server::AppServer;
use roder_core::{Runtime, fake_provider::FakeInferenceEngine};
use roder_ext_gbrain::{CaptureInput, Embedder, GbrainStore, GbrainStoreFactory};
use roder_protocol::{GbrainGraphResult, GbrainStatusResult, JsonRpcRequest, JsonRpcResponse};
use serde_json::json;
use time::OffsetDateTime;

fn temp_root(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "roder-gbrain-bridge-{name}-{}",
        uuid::Uuid::new_v4()
    ))
}

async fn fixture_server(name: &str) -> (AppServer, std::path::PathBuf) {
    let base = temp_root(name);
    let db_path = base.join("gbrain.sqlite3");
    let store = GbrainStore::open(db_path.clone(), Embedder::new(None)).unwrap();

    let mut first = CaptureInput::new(
        MemoryScope::Project("helix".to_string()),
        "Helix moved the billing owner to Mina for the retention recovery work.",
    );
    first.subject = Some("Helix billing owner".to_string());
    first.valid_at = Some(OffsetDateTime::UNIX_EPOCH);
    first.provenance = vec!["artifact://helix/retention.md".to_string()];
    store.capture(first).await.unwrap();

    let mut second = CaptureInput::new(
        MemoryScope::Project("helix".to_string()),
        "The retention recovery decision superseded the older billing rotation note.",
    );
    second.subject = Some("Helix retention decision".to_string());
    second.valid_at = Some(OffsetDateTime::UNIX_EPOCH);
    second.provenance = vec!["artifact://helix/decision.md".to_string()];
    store.capture(second).await.unwrap();
    drop(store);

    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FakeInferenceEngine));
    builder.memory_store_factory(Arc::new(GbrainStoreFactory::new(base.clone(), None)));
    let runtime = Arc::new(Runtime::new(builder.build().unwrap(), Default::default()).unwrap());
    (AppServer::new(runtime), db_path)
}

async fn call(server: &AppServer, method: &str, params: serde_json::Value) -> JsonRpcResponse {
    server
        .handle_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(1)),
            method: method.to_string(),
            params: Some(params),
        })
        .await
}

#[tokio::test]
async fn gbrain_status_reports_registered_store() {
    let (server, db_path) = fixture_server("status").await;

    let response = call(
        &server,
        "gbrain/status",
        json!({ "scope": "project:helix" }),
    )
    .await;
    assert!(response.error.is_none(), "{:?}", response.error);
    let result: GbrainStatusResult = serde_json::from_value(response.result.unwrap()).unwrap();

    assert!(result.available);
    assert_eq!(result.scope_id, "project:helix");
    assert_eq!(result.store_path, db_path.display().to_string());
    assert_eq!(result.stats.raw_fact_count, 2);
}

#[tokio::test]
async fn gbrain_graph_falls_back_to_raw_facts_without_dreamed_nodes() {
    let (server, _) = fixture_server("fallback").await;

    let response = call(
        &server,
        "gbrain/graph",
        json!({
            "scope": "project:helix",
            "query": "retention",
            "limit": 20,
            "includeEvidence": true
        }),
    )
    .await;
    assert!(response.error.is_none(), "{:?}", response.error);
    let result: GbrainGraphResult = serde_json::from_value(response.result.unwrap()).unwrap();

    assert!(result.stats.fallback_raw);
    assert_eq!(result.scope_id, "project:helix");
    assert!(!result.nodes.is_empty());
    assert!(result.nodes.iter().all(|node| node.kind == "raw_fact"));
    assert!(
        result
            .nodes
            .iter()
            .any(|node| node.source_fact_id.is_some())
    );
}

#[tokio::test]
async fn gbrain_graph_prefers_derived_nodes_when_available() {
    let (server, db_path) = fixture_server("derived").await;
    let conn = rusqlite::Connection::open(db_path).unwrap();
    conn.execute(
        "INSERT INTO gbrain_dream_runs(id, scope_id, mode, started_at, status, algorithm_version, run_policy)
         VALUES ('dream-run-1', 'project:helix', 'full', '1970-01-01T00:00:00Z', 'completed', 'test', 'maintenance')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO gbrain_nodes(id, label, node_kind, scope_id, confidence, active, created_by_run_id, created_at)
         VALUES ('node:helix:owner', 'Helix billing owner', 'person_role', 'project:helix', 'INFERRED', 1, 'dream-run-1', '1970-01-01T00:00:00Z')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO gbrain_nodes(id, label, node_kind, scope_id, confidence, active, created_by_run_id, created_at)
         VALUES ('node:helix:retention', 'Retention recovery work', 'initiative', 'project:helix', 'EXTRACTED', 1, 'dream-run-1', '1970-01-01T00:00:00Z')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO gbrain_edges(id, source_node_id, target_node_id, relation, confidence, directed, evidence_ids, active, dream_run_id, created_at)
         VALUES ('edge:helix:owner-retention', 'node:helix:owner', 'node:helix:retention', 'owns', 'INFERRED', 1, '[]', 1, 'dream-run-1', '1970-01-01T00:00:00Z')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO gbrain_evidence_cards(id, scope_id, dream_run_id, title, summary, quote_spans, source_fact_ids, temporal_status, neighboring_event_ids, confidence, active, created_at)
         VALUES ('evidence:helix:owner', 'project:helix', 'dream-run-1', 'Owner evidence', 'Mina owns the recovery work.', '[]', '[]', 'active', '[]', 'EXTRACTED', 1, '1970-01-01T00:00:00Z')",
        [],
    )
    .unwrap();
    drop(conn);

    let response = call(
        &server,
        "gbrain/graph",
        json!({
            "scope": "project:helix",
            "limit": 20,
            "includeEvidence": true
        }),
    )
    .await;
    assert!(response.error.is_none(), "{:?}", response.error);
    let result: GbrainGraphResult = serde_json::from_value(response.result.unwrap()).unwrap();

    assert!(!result.stats.fallback_raw);
    assert_eq!(result.nodes.len(), 2);
    assert_eq!(result.edges.len(), 1);
    assert_eq!(result.evidence_cards.len(), 1);
    assert_eq!(result.dream_runs.len(), 1);
}

#[tokio::test]
async fn gbrain_search_and_node_return_projected_graph() {
    let (server, _) = fixture_server("search-node").await;

    let search_response = call(
        &server,
        "gbrain/search",
        json!({
            "scope": "project:helix",
            "query": "billing",
            "limit": 5
        }),
    )
    .await;
    assert!(
        search_response.error.is_none(),
        "{:?}",
        search_response.error
    );
    let graph: GbrainGraphResult = serde_json::from_value(search_response.result.unwrap()).unwrap();
    let node_id = graph.nodes.first().expect("search node").id.clone();

    let node_response = call(
        &server,
        "gbrain/node",
        json!({
            "scope": "project:helix",
            "nodeId": node_id
        }),
    )
    .await;
    assert!(node_response.error.is_none(), "{:?}", node_response.error);
    assert!(node_response.result.unwrap()["node"].is_object());
}
