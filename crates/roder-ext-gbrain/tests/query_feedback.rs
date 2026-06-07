use std::path::PathBuf;
use std::sync::Arc;

use roder_api::memory::MemoryScope;
use roder_api::tools::ToolRegistry;
use roder_ext_gbrain::agent::retriever::{
    AgenticRetriever, FakeToolPlanner, ModelSelectedToolCall, ProviderTurn,
    QueryFeedbackTraceMetadata,
};
use roder_ext_gbrain::store::QueryFeedbackInput;
use roder_ext_gbrain::{CaptureInput, Embedder, GbrainStore, GbrainToolContributor};
use rusqlite::Connection;
use serde_json::json;

fn temp_db_path(test_name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "roder-ext-gbrain-{test_name}-{}.sqlite3",
        uuid::Uuid::new_v4()
    ))
}

fn read_only_registry(store: Arc<GbrainStore>) -> ToolRegistry {
    let mut registry = ToolRegistry::default();
    GbrainToolContributor::new(store)
        .contribute_read_only(&mut registry)
        .unwrap();
    registry
}

fn count_request_time_shape_rows(path: &PathBuf) -> i64 {
    let conn = Connection::open(path).unwrap();
    conn.query_row(
        "SELECT
            (SELECT COUNT(*) FROM gbrain_ontology_nodes WHERE active = 1)
          + (SELECT COUNT(*) FROM gbrain_relevance)
          + (SELECT COUNT(*) FROM gbrain_communities WHERE active = 1)
          + (SELECT COUNT(*) FROM gbrain_evidence_cards WHERE active = 1)
          + (SELECT COUNT(*) FROM gbrain_dream_runs)
          + (SELECT COUNT(*) FROM gbrain_nodes WHERE active = 1)",
        [],
        |row| row.get(0),
    )
    .unwrap()
}

#[tokio::test]
async fn appends_agentic_answer_feedback_without_reshaping_memory() {
    let db_path = temp_db_path("append-agentic-answer-feedback");
    let store = Arc::new(GbrainStore::open(db_path.clone(), Embedder::new(None)).unwrap());
    let scope = MemoryScope::Project("helix".to_string());
    store
        .capture(CaptureInput::new(
            scope.clone(),
            "Maya owns the Acme account as of 2024-01-01.",
        ))
        .await
        .unwrap();
    let shape_rows_before = count_request_time_shape_rows(&db_path);
    let retriever = AgenticRetriever::new(read_only_registry(store.clone()));
    let mut planner = FakeToolPlanner::new([
        ProviderTurn::ToolCalls(vec![ModelSelectedToolCall::new(
            "start-1",
            "gbrain_find_start_nodes",
            json!({"query": "Acme owner", "scope": "project:helix", "limit": 3}),
        )]),
        ProviderTurn::ToolCalls(vec![ModelSelectedToolCall::new(
            "note-1",
            "gbrain_retrieval_note",
            json!({
                "note": "Acme ownership answer has one raw-fact start node.",
                "evidenceIds": ["card:acme-ownership-card", "event:acme-ownership-event"],
            }),
        )]),
        ProviderTurn::FinalResponse("Maya owns the Acme account.".to_string()),
    ]);

    let trace = retriever
        .run_with_query_feedback_metadata(
            "Who owns Acme?",
            &mut planner,
            QueryFeedbackTraceMetadata {
                question_kind: Some("owner_lookup".to_string()),
                eval_result_id: Some("eval-result-42".to_string()),
            },
        )
        .await
        .unwrap();
    let feedback = trace.to_query_feedback_input(Some(scope.clone()), Some(17));
    let appended = store.append_query_feedback(feedback).await.unwrap();
    let rows = store
        .load_query_feedback(Some(scope.clone()))
        .await
        .unwrap();

    assert_eq!(rows, vec![appended.clone()]);
    assert_eq!(appended.scope_id.as_deref(), Some("project:helix"));
    assert_eq!(appended.question, "Who owns Acme?");
    assert_eq!(appended.question_kind.as_deref(), Some("owner_lookup"));
    assert_eq!(appended.tool_call_count, 2);
    assert_eq!(appended.stop_reason.as_deref(), Some("final_response"));
    assert_eq!(appended.duration_ms, Some(17));
    assert_eq!(
        appended.answer_length,
        Some("Maya owns the Acme account.".len())
    );
    assert!(appended.response_hash.is_some());
    assert_eq!(appended.eval_result_id.as_deref(), Some("eval-result-42"));
    assert!(appended.used_nodes.len() == 1, "{appended:#?}");
    assert_eq!(
        appended.used_cards,
        vec!["card:acme-ownership-card".to_string()]
    );
    assert_eq!(
        appended.used_events,
        vec!["event:acme-ownership-event".to_string()]
    );
    assert_eq!(count_request_time_shape_rows(&db_path), shape_rows_before);

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn direct_feedback_append_is_auditable_and_append_only() {
    let store = GbrainStore::open_in_memory(Embedder::new(None)).unwrap();
    let first = store
        .append_query_feedback(QueryFeedbackInput {
            scope: None,
            question: "What changed?".to_string(),
            question_kind: Some("change_lookup".to_string()),
            used_nodes: vec!["node:one".to_string()],
            used_cards: Vec::new(),
            used_events: vec!["event:one".to_string()],
            duration_ms: None,
            tool_call_count: 3,
            stop_reason: Some("final_response".to_string()),
            answer_length: Some(12),
            response_hash: Some("hash-one".to_string()),
            eval_result_id: None,
        })
        .await
        .unwrap();
    let second = store
        .append_query_feedback(QueryFeedbackInput {
            scope: None,
            question: "What changed?".to_string(),
            question_kind: Some("change_lookup".to_string()),
            used_nodes: vec!["node:two".to_string()],
            used_cards: Vec::new(),
            used_events: Vec::new(),
            duration_ms: None,
            tool_call_count: 1,
            stop_reason: Some("abstained".to_string()),
            answer_length: None,
            response_hash: None,
            eval_result_id: Some("eval-two".to_string()),
        })
        .await
        .unwrap();

    let rows = store.load_query_feedback(None).await.unwrap();

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0], first);
    assert_eq!(rows[1], second);
}
