use std::sync::Arc;

use roder_api::policy_mode::PolicyMode;
use roder_api::tools::{ToolCall, ToolContributor, ToolExecutionContext, ToolRegistry};
use roder_ext_knowledge_md::MarkdownKnowledgeStore;
use roder_ext_knowledge_md::tools::KnowledgeToolContributor;
use serde_json::json;

fn registry() -> ToolRegistry {
    let base = std::env::temp_dir().join(format!("roder-knowledge-{}", uuid::Uuid::new_v4()));
    let store = Arc::new(MarkdownKnowledgeStore::new(base));
    let contributor = KnowledgeToolContributor::new(store);
    let mut registry = ToolRegistry::default();
    contributor.contribute(&mut registry).unwrap();
    registry
}

fn context() -> ToolExecutionContext {
    ToolExecutionContext::new("thread-a", "turn-a", PolicyMode::Default)
}

fn call(name: &str, arguments: serde_json::Value) -> ToolCall {
    ToolCall {
        id: format!("call-{name}"),
        name: name.to_string(),
        raw_arguments: arguments.to_string(),
        arguments,
        thread_id: "thread-a".to_string(),
        turn_id: "turn-a".to_string(),
    }
}

async fn execute(
    registry: &ToolRegistry,
    name: &str,
    arguments: serde_json::Value,
) -> roder_api::tools::ToolResult {
    registry
        .get(name)
        .unwrap_or_else(|| panic!("tool {name} not registered"))
        .execute(context(), call(name, arguments))
        .await
        .unwrap()
}

#[tokio::test]
async fn all_knowledge_tools_are_registered() {
    let registry = registry();
    for tool in [
        "knowledge_list",
        "knowledge_read",
        "knowledge_search",
        "knowledge_save",
        "knowledge_update",
        "knowledge_delete",
        "knowledge_link",
    ] {
        assert!(registry.get(tool).is_some(), "missing tool {tool}");
    }
}

#[tokio::test]
async fn save_list_search_read_round_trip_through_tools() {
    let registry = registry();

    let saved = execute(
        &registry,
        "knowledge_save",
        json!({
            "kind": "decision",
            "title": "Adopt markdown knowledge",
            "body": "We adopt a markdown knowledge base with the codeword osprey.",
            "tags": ["adr"],
            "scope": "project:demo"
        }),
    )
    .await;
    let id = saved.data["document"]["id"].as_str().unwrap().to_string();
    assert!(saved.text.contains("saved knowledge document"));

    let listed = execute(
        &registry,
        "knowledge_list",
        json!({ "scope": "project:demo", "kind": "decision" }),
    )
    .await;
    assert!(listed.text.contains("Adopt markdown knowledge"));
    assert!(listed.text.contains(&id));

    let searched = execute(
        &registry,
        "knowledge_search",
        json!({ "query": "osprey", "scope": "project:demo" }),
    )
    .await;
    assert!(searched.text.contains(&id));
    assert!(searched.text.contains("osprey"));

    let read = execute(&registry, "knowledge_read", json!({ "id": id })).await;
    assert!(read.text.contains("# Adopt markdown knowledge"));
    assert!(read.text.contains("osprey"));
}

#[tokio::test]
async fn read_paginates_long_documents() {
    let registry = registry();
    let body = (1..=450)
        .map(|n| format!("line {n}"))
        .collect::<Vec<_>>()
        .join("\n");
    let saved = execute(
        &registry,
        "knowledge_save",
        json!({
            "kind": "research",
            "title": "Long doc",
            "body": body,
            "scope": "project:demo"
        }),
    )
    .await;
    let id = saved.data["document"]["id"].as_str().unwrap().to_string();

    let first = execute(&registry, "knowledge_read", json!({ "id": id })).await;
    assert!(first.text.contains("line 200"));
    assert!(!first.text.contains("line 201\n"));
    assert!(first.text.contains("offset=200"));

    let second = execute(
        &registry,
        "knowledge_read",
        json!({ "id": id, "offset": 200 }),
    )
    .await;
    assert!(second.text.contains("line 201"));
    assert!(second.text.contains("line 400"));
    assert!(second.text.contains("offset=400"));
}

#[tokio::test]
async fn update_link_and_delete_through_tools() {
    let registry = registry();
    let first = execute(
        &registry,
        "knowledge_save",
        json!({ "kind": "note", "title": "First", "body": "a", "scope": "project:demo" }),
    )
    .await;
    let second = execute(
        &registry,
        "knowledge_save",
        json!({ "kind": "note", "title": "Second", "body": "b", "scope": "project:demo" }),
    )
    .await;
    let first_id = first.data["document"]["id"].as_str().unwrap().to_string();
    let second_id = second.data["document"]["id"].as_str().unwrap().to_string();

    let updated = execute(
        &registry,
        "knowledge_update",
        json!({ "id": first_id, "body": "a v2", "status": "draft" }),
    )
    .await;
    assert!(updated.text.contains("revision 2"));

    let linked = execute(
        &registry,
        "knowledge_link",
        json!({ "from": second_id, "to": first_id, "type": "supersedes" }),
    )
    .await;
    assert!(linked.text.contains("set link"));

    let deleted = execute(&registry, "knowledge_delete", json!({ "id": first_id })).await;
    assert!(deleted.data["archived"].as_bool().unwrap());

    let listed = execute(
        &registry,
        "knowledge_list",
        json!({ "scope": "project:demo" }),
    )
    .await;
    assert!(!listed.text.contains(&first_id));
    assert!(listed.text.contains(&second_id));
}

#[tokio::test]
async fn unknown_link_type_fails_with_guidance() {
    let registry = registry();
    let error = registry
        .get("knowledge_link")
        .unwrap()
        .execute(
            context(),
            call(
                "knowledge_link",
                json!({ "from": "kn-a", "to": "kn-b", "type": "blocks" }),
            ),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("unknown link type"));
}
