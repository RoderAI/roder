//! Offline stream fixtures for OpenAI Responses provider-native tool search.
//!
//! These tests feed recorded `tool_search_call` SSE shapes through the same
//! state machine used by the live stream and assert the canonical Roder
//! hosted/tool-call lifecycle, so vendor payload changes are caught offline.

use super::*;
use roder_api::inference::{HostedToolCallCompleted, HostedToolCallStarted};

fn sse(event: &str, data: Value) -> SseEvent {
    SseEvent {
        event: Some(event.to_string()),
        data,
    }
}

#[test]
fn maps_tool_search_call_items_to_hosted_tool_lifecycle() {
    let mut state = ResponsesStreamState::default();

    let added = sse(
        "response.output_item.added",
        json!({
            "type": "response.output_item.added",
            "item": {
                "id": "ts_1",
                "type": "tool_search_call",
                "status": "in_progress",
                "query": "edit files"
            }
        }),
    );
    assert_eq!(
        events_from_sse_event(&added, &mut state),
        vec![InferenceEvent::HostedToolCallStarted(
            HostedToolCallStarted {
                id: "ts_1".to_string(),
                name: "tool_search".to_string(),
            }
        )]
    );

    let done = sse(
        "response.output_item.done",
        json!({
            "type": "response.output_item.done",
            "item": {
                "id": "ts_1",
                "type": "tool_search_call",
                "status": "completed",
                "query": "edit files",
                "results": [
                    { "name": "edit_file" },
                    { "tool_name": "read_file" }
                ]
            }
        }),
    );
    let events = events_from_sse_event(&done, &mut state);
    assert_eq!(events.len(), 1);
    let InferenceEvent::HostedToolCallCompleted(completed) = &events[0] else {
        panic!("expected hosted tool completion, got {events:?}");
    };
    assert_eq!(completed.id, "ts_1");
    assert_eq!(completed.name, "tool_search");
    let arguments: Value = serde_json::from_str(&completed.arguments).unwrap();
    assert_eq!(arguments["query"], "edit files");
    assert_eq!(
        arguments["selected_tools"],
        json!(["edit_file", "read_file"])
    );
}

#[test]
fn searched_tool_selection_executes_through_normal_tool_call_lifecycle() {
    let mut state = ResponsesStreamState::default();

    let search_added = sse(
        "response.output_item.added",
        json!({
            "type": "response.output_item.added",
            "item": { "id": "ts_1", "type": "tool_search_call", "query": "edit" }
        }),
    );
    events_from_sse_event(&search_added, &mut state);

    let tool_added = sse(
        "response.output_item.added",
        json!({
            "type": "response.output_item.added",
            "item": {
                "id": "item_fc1",
                "type": "function_call",
                "call_id": "call_1",
                "name": "edit_file"
            }
        }),
    );
    assert_eq!(
        events_from_sse_event(&tool_added, &mut state),
        vec![InferenceEvent::ToolCallStarted(ToolCallStarted {
            id: "call_1".to_string(),
            name: "edit_file".to_string(),
        })]
    );

    let args_done = sse(
        "response.function_call_arguments.done",
        json!({
            "type": "response.function_call_arguments.done",
            "item_id": "item_fc1",
            "name": "edit_file",
            "arguments": "{\"path\":\"a.txt\"}"
        }),
    );
    assert_eq!(
        events_from_sse_event(&args_done, &mut state),
        vec![InferenceEvent::ToolCallCompleted(ToolCallCompleted {
            id: "call_1".to_string(),
            name: "edit_file".to_string(),
            arguments: "{\"path\":\"a.txt\"}".to_string(),
        })]
    );
}

#[test]
fn completed_response_recovers_unstreamed_tool_search_call_items() {
    let mut state = ResponsesStreamState::default();
    let completed = sse(
        "response.completed",
        json!({
            "type": "response.completed",
            "response": {
                "id": "resp_1",
                "status": "completed",
                "output": [
                    {
                        "id": "ts_9",
                        "type": "tool_search_call",
                        "query": "git status",
                        "results": [{ "name": "git_status" }]
                    }
                ]
            }
        }),
    );

    let events = events_from_sse_event(&completed, &mut state);

    assert!(events.iter().any(|event| matches!(
        event,
        InferenceEvent::HostedToolCallStarted(HostedToolCallStarted { id, name })
            if id == "ts_9" && name == "tool_search"
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        InferenceEvent::HostedToolCallCompleted(HostedToolCallCompleted { id, name, arguments })
            if id == "ts_9"
                && name == "tool_search"
                && arguments.contains("git_status")
    )));
    assert!(state.terminal);
}

#[test]
fn web_search_call_arguments_keep_existing_action_shape() {
    let item = json!({
        "id": "ws_1",
        "type": "web_search_call",
        "action": { "type": "search", "query": "weather" }
    });
    let call = hosted_tool_call_completed_from_item(&item).unwrap();
    assert_eq!(call.name, "web_search");
    let arguments: Value = serde_json::from_str(&call.arguments).unwrap();
    assert_eq!(arguments["action"], "search");
    assert_eq!(arguments["query"], "weather");
}

#[test]
fn client_executed_tool_search_items_pend_without_completing() {
    let mut state = ResponsesStreamState::default();

    // No `results` and a query: the client must execute the search.
    let done = sse(
        "response.output_item.done",
        json!({
            "type": "response.output_item.done",
            "item": {
                "id": "ts_7",
                "type": "tool_search_call",
                "status": "completed",
                "query": "deploy"
            }
        }),
    );
    let events = events_from_sse_event(&done, &mut state);
    assert_eq!(
        events,
        vec![InferenceEvent::HostedToolCallStarted(
            HostedToolCallStarted {
                id: "ts_7".to_string(),
                name: "tool_search".to_string(),
            }
        )],
        "no completion is emitted before the local search runs"
    );
    assert_eq!(state.pending_client_tool_searches.len(), 1);

    // response.completed recovery does not duplicate the pending item and
    // does not surface it as a hosted completion.
    let completed = sse(
        "response.completed",
        json!({
            "type": "response.completed",
            "response": {
                "id": "resp_1",
                "status": "completed",
                "output": [{
                    "id": "ts_7",
                    "type": "tool_search_call",
                    "status": "completed",
                    "query": "deploy"
                }]
            }
        }),
    );
    let events = events_from_sse_event(&completed, &mut state);
    assert_eq!(state.pending_client_tool_searches.len(), 1);
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, InferenceEvent::HostedToolCallCompleted(_))),
        "{events:?}"
    );

    // Hosted (server-executed) searches with results still complete inline.
    let mut hosted_state = ResponsesStreamState::default();
    let hosted = sse(
        "response.output_item.done",
        json!({
            "type": "response.output_item.done",
            "item": {
                "id": "ts_8",
                "type": "tool_search_call",
                "status": "completed",
                "query": "deploy",
                "results": [{ "name": "deploy_app" }]
            }
        }),
    );
    let events = events_from_sse_event(&hosted, &mut hosted_state);
    assert!(
        events
            .iter()
            .any(|event| matches!(event, InferenceEvent::HostedToolCallCompleted(_)))
    );
    assert!(hosted_state.pending_client_tool_searches.is_empty());
}
