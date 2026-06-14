//! Offline stream fixtures for Anthropic provider-native tool search.
//!
//! These tests replay the documented Claude tool-search block sequence
//! (`server_tool_use` -> `tool_search_tool_result` with `tool_reference`
//! entries -> normal `tool_use` of a discovered tool) and assert that the
//! discovered tool executes through the same canonical Roder tool-call
//! lifecycle as explicit tools, while search metadata stays available in the
//! reconstructed ProviderMetadata message.

use super::*;

fn frames(state: &mut AnthropicStreamState, frames: &[&str]) -> Vec<InferenceEvent> {
    frames
        .iter()
        .flat_map(|frame| state.push_frame(frame).unwrap())
        .collect()
}

fn search_selection_frames() -> Vec<&'static str> {
    vec![
        r#"data: {"type":"message_start","message":{"id":"msg_ts","usage":{"input_tokens":10}}}"#,
        // Provider-side tool search call.
        r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"server_tool_use","id":"srvtoolu_1","name":"tool_search","input":{}}}"#,
        r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"query\":\"edit"}}"#,
        r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":" files\"}"}}"#,
        r#"data: {"type":"content_block_stop","index":0}"#,
        // Search results referencing discovered tools.
        r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_search_tool_result","tool_use_id":"srvtoolu_1","content":[{"type":"tool_search_tool_search_result","tool_references":[{"type":"tool_reference","name":"edit_file"},{"type":"tool_reference","name":"read_file"}]}]}}"#,
        r#"data: {"type":"content_block_stop","index":1}"#,
        // The model then calls a discovered tool like any explicit tool.
        r#"data: {"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"toolu_sel","name":"edit_file","input":{}}}"#,
        r#"data: {"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"a.txt\"}"}}"#,
        r#"data: {"type":"content_block_stop","index":2}"#,
        r#"data: {"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":12}}"#,
        r#"data: {"type":"message_stop"}"#,
    ]
}

#[test]
fn tool_search_selection_executes_through_canonical_tool_call_lifecycle() {
    let tools = vec![ToolSpec {
        name: "edit_file".to_string(),
        description: "edit a file".to_string(),
        parameters: json!({ "type": "object" }),
    }];
    let mut state = AnthropicStreamState::new(tools);

    let events = frames(&mut state, &search_selection_frames());

    assert!(events.contains(&InferenceEvent::HostedToolCallStarted(
        HostedToolCallStarted {
            id: "srvtoolu_1".to_string(),
            name: "tool_search".to_string(),
        }
    )));
    assert!(events.contains(&InferenceEvent::HostedToolCallCompleted(
        HostedToolCallCompleted {
            id: "srvtoolu_1".to_string(),
            name: "tool_search".to_string(),
            arguments: r#"{"query":"edit files"}"#.to_string(),
        }
    )));
    // Discovered tool selection flows through the normal tool-call lifecycle.
    assert!(
        events.contains(&InferenceEvent::ToolCallStarted(ToolCallStarted {
            id: "toolu_sel".to_string(),
            name: "edit_file".to_string(),
        }))
    );
    assert!(
        events.contains(&InferenceEvent::ToolCallCompleted(ToolCallCompleted {
            id: "toolu_sel".to_string(),
            name: "edit_file".to_string(),
            arguments: r#"{"path":"a.txt"}"#.to_string(),
        }))
    );
    assert!(matches!(
        events.last(),
        Some(InferenceEvent::Completed(CompletionMetadata {
            stop_reason: Some(reason),
            ..
        })) if reason == "tool_use"
    ));
}

#[test]
fn tool_search_result_blocks_are_preserved_in_provider_metadata() {
    let mut state = AnthropicStreamState::new(Vec::new());
    let events = frames(&mut state, &search_selection_frames());

    let metadata = events
        .iter()
        .find_map(|event| match event {
            InferenceEvent::ProviderMetadata(value) => Some(value.clone()),
            _ => None,
        })
        .expect("provider metadata event");
    let content = metadata["content"].as_array().expect("content array");

    assert_eq!(content[0]["type"], "server_tool_use");
    assert_eq!(content[0]["input"], json!({ "query": "edit files" }));
    assert_eq!(content[1]["type"], "tool_search_tool_result");
    assert_eq!(
        content[1]["content"][0]["tool_references"],
        json!([
            { "type": "tool_reference", "name": "edit_file" },
            { "type": "tool_reference", "name": "read_file" }
        ])
    );
    assert_eq!(content[2]["type"], "tool_use");
}

#[test]
fn non_tool_search_server_tool_use_blocks_still_emit_no_events() {
    let mut state = AnthropicStreamState::new(Vec::new());
    let events = frames(
        &mut state,
        &[
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"server_tool_use","id":"srv_ws","name":"web_search","input":{}}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"query\":\"weather\"}"}}"#,
            r#"data: {"type":"content_block_stop","index":0}"#,
        ],
    );

    assert_eq!(events, Vec::new());
    assert_eq!(state.message["content"][0]["type"], "server_tool_use");
    assert_eq!(state.message["content"][0]["name"], "web_search");
    assert_eq!(
        state.message["content"][0]["input"],
        json!({ "query": "weather" })
    );
}
