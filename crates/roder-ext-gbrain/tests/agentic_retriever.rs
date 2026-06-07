use std::sync::Arc;

use roder_api::memory::MemoryScope;
use roder_api::policy_mode::PolicyMode;
use roder_api::tools::{ToolCall, ToolExecutionContext, ToolRegistry, ToolResult};
use roder_ext_gbrain::agent::prompts::agentic_retrieval_system_prompt;
use roder_ext_gbrain::agent::retriever::{
    AgenticRetriever, AgenticRetrieverConfig, FakeToolPlanner, ModelSelectedToolCall, ProviderTurn,
};
use roder_ext_gbrain::{
    CaptureInput, DreamMode, DreamParams, DreamPolicy, Embedder, GbrainStore,
    GbrainToolContributor, is_read_only_tool, read_only_tool_names,
};
use serde_json::json;

fn store() -> Arc<GbrainStore> {
    Arc::new(GbrainStore::open_in_memory(Embedder::new(None)).unwrap())
}

fn read_only_registry(store: Arc<GbrainStore>) -> ToolRegistry {
    let mut registry = ToolRegistry::default();
    GbrainToolContributor::new(store)
        .contribute_read_only(&mut registry)
        .unwrap();
    registry
}

async fn run_tool(registry: &ToolRegistry, name: &str, args: serde_json::Value) -> ToolResult {
    let tool = registry.get(name).expect("registered tool");
    tool.execute(
        ToolExecutionContext::new("agentic-retriever-test", "turn-1", PolicyMode::Default),
        ToolCall {
            id: format!("call-{name}"),
            name: name.to_string(),
            raw_arguments: args.to_string(),
            arguments: args,
            thread_id: "agentic-retriever-test".to_string(),
            turn_id: "turn-1".to_string(),
        },
    )
    .await
    .unwrap()
}

#[test]
fn read_only_tool_guard_excludes_mutating_ops() {
    assert!(is_read_only_tool("gbrain_search_raw"));
    assert!(is_read_only_tool("gbrain_find_paths"));
    assert!(is_read_only_tool("respond_to_query"));
    assert!(!is_read_only_tool("gbrain_capture"));
    assert!(!is_read_only_tool("gbrain_supersede"));
    assert!(!is_read_only_tool("gbrain_consolidate"));

    let names = read_only_tool_names();
    assert!(names.contains(&"gbrain_find_start_nodes"));
    assert!(!names.contains(&"gbrain_capture"));
}

#[test]
fn read_only_contributor_registers_only_read_only_specs() {
    let registry = read_only_registry(store());
    let names: Vec<String> = registry.specs().into_iter().map(|spec| spec.name).collect();

    assert!(names.contains(&"gbrain_search_raw".to_string()));
    assert!(names.contains(&"gbrain_find_contradictions".to_string()));
    assert!(names.contains(&"gbrain_expand_neighbors".to_string()));
    assert!(names.contains(&"respond_to_query".to_string()));
    assert!(!names.contains(&"gbrain_capture".to_string()));
    assert!(!names.contains(&"gbrain_supersede".to_string()));
    assert!(!names.contains(&"gbrain_consolidate".to_string()));
}

#[tokio::test]
async fn read_only_tools_return_structured_observations() {
    let store = store();
    store
        .capture(CaptureInput::new(
            MemoryScope::Global,
            "Maya owns the Acme account as of 2024-01-01.",
        ))
        .await
        .unwrap();
    let retriever = AgenticRetriever::new(read_only_registry(store));
    let mut planner = FakeToolPlanner::new([
        ProviderTurn::ToolCalls(vec![ModelSelectedToolCall::new(
            "raw-1",
            "gbrain_search_raw",
            json!({"query": "Acme owner", "limit": 5}),
        )]),
        ProviderTurn::ToolCalls(vec![ModelSelectedToolCall::new(
            "graph-1",
            "gbrain_expand_neighbors",
            json!({"nodeId": "missing-node", "depth": 1}),
        )]),
        ProviderTurn::FinalResponse("Maya owns Acme based on the available record.".to_string()),
    ]);

    let trace = retriever.run("Who owns Acme?", &mut planner).await.unwrap();

    assert_eq!(
        trace.final_response.as_deref(),
        Some("Maya owns Acme based on the available record.")
    );
    assert_eq!(trace.observations.len(), 2);
    assert_eq!(trace.observations[0].result.data["readOnly"], true);
    assert_eq!(
        trace.observations[0].result.data["observations"][0]["observationType"],
        "raw_fact"
    );
    assert_eq!(
        trace.observations[1].result.data["status"],
        "not_yet_dreamed"
    );
}

#[tokio::test]
async fn graph_tools_use_dreamed_rows_before_raw_fallback() {
    let store = store();
    let scope = MemoryScope::Project("helix".to_string());
    for text in [
        "Maya owns the Acme account as of 2024-01-01.",
        "The Acme account was discussed in thread thread-acme.",
    ] {
        let mut input = CaptureInput::new(scope.clone(), text);
        input.metadata = json!({"thread_id": "thread-acme"});
        store.capture(input).await.unwrap();
    }
    store
        .dream(DreamParams {
            mode: DreamMode::Enrich,
            scope: scope.clone(),
            since: None,
            run_policy: DreamPolicy::Eval,
            workers: 1,
            dry_run: false,
            cancellation_token: None,
            reasoner_model: None,
        })
        .await
        .unwrap();

    let registry = read_only_registry(store);
    let start = run_tool(
        &registry,
        "gbrain_find_start_nodes",
        json!({"query": "Acme owner", "scope": "project:helix", "limit": 5}),
    )
    .await;
    assert_eq!(
        start.data["observations"][0]["observationType"],
        "dream_start_node"
    );
    assert_eq!(start.data["trace"]["fallback"], false);
    let node_id = start.data["observations"][0]["node"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let explain = run_tool(&registry, "gbrain_explain_node", json!({"nodeId": node_id})).await;
    assert_eq!(
        explain.data["observations"][0]["observationType"],
        "dream_node_explanation"
    );
    assert!(
        !explain.data["observations"][0]["evidenceCards"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let neighbors = run_tool(
        &registry,
        "gbrain_expand_neighbors",
        json!({"nodeId": node_id, "depth": 1}),
    )
    .await;
    assert_eq!(neighbors.data["trace"]["fallback"], false);
}

#[tokio::test]
async fn fake_provider_loop_continues_past_old_max_subqueries_style_cap() {
    let retriever =
        AgenticRetriever::new(read_only_registry(store())).with_config(AgenticRetrieverConfig {
            max_tool_calls: Some(8),
        });
    let mut turns = Vec::new();
    for i in 0..5 {
        turns.push(ProviderTurn::ToolCalls(vec![ModelSelectedToolCall::new(
            format!("note-{i}"),
            "gbrain_retrieval_note",
            json!({"note": format!("still checking evidence pass {i}")}),
        )]));
    }
    turns.push(ProviderTurn::FinalResponse(
        "Final free-form answer after five model-selected tool turns.".to_string(),
    ));
    let mut planner = FakeToolPlanner::new(turns);

    let trace = retriever
        .run("Continue until evidence is sufficient.", &mut planner)
        .await
        .unwrap();

    assert_eq!(trace.observations.len(), 5);
    assert_eq!(
        trace.final_response.as_deref(),
        Some("Final free-form answer after five model-selected tool turns.")
    );
}

#[tokio::test]
async fn fake_provider_loop_rejects_mutating_tools() {
    let retriever = AgenticRetriever::new(read_only_registry(store()));
    let mut planner =
        FakeToolPlanner::new([ProviderTurn::ToolCalls(vec![ModelSelectedToolCall::new(
            "capture-1",
            "gbrain_capture",
            json!({"text": "do not write"}),
        )])]);

    let err = retriever
        .run("Try to mutate during retrieval.", &mut planner)
        .await
        .unwrap_err();

    assert!(err.to_string().contains("non-read-only tool"));
}

#[test]
fn prompt_guidance_mentions_required_retrieval_checks() {
    let prompt = agentic_retrieval_system_prompt();
    assert!(prompt.contains("actor, action, date"));
    assert!(prompt.contains("contradiction"));
    assert!(prompt.contains("as-of"));
    assert!(prompt.contains("Abstain"));
}
