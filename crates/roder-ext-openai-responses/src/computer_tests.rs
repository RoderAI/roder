use super::*;
use roder_api::computer::computer_tool_spec;
use roder_api::transcript::{ToolCallRecord, ToolResultRecord, TranscriptItem};

fn native_item() -> Value {
    json!({"id":"cu_1","type":"computer_call","call_id":"call_native",
        "actions":[{"type":"click","button":"left","x":0,"y":0},{"type":"type","text":"penguin"}],"status":"completed"})
}
fn native_request() -> AgentInferenceRequest {
    let mut request = tests::request();
    request.tools = vec![computer_tool_spec()];
    request.tool_choice = roder_api::tools::ToolChoice::Specific("computer".into());
    request
}
fn add_result(request: &mut AgentInferenceRequest, is_error: bool) {
    request.transcript.push(TranscriptItem::ToolResult(ToolResultRecord {
        id: "call_native".into(), name: Some("computer".into()), result: "Observed current screen".into(),
        display_payload: Some(json!({"__view_image":{"image_url":"data:image/jpeg;base64,YWJj","detail":"original"}})), is_error,
    }));
}
#[test]
fn native_registration_and_forced_choice_do_not_create_a_function_or_defer_computer() {
    let mut request = native_request();
    request.runtime.tool_search = roder_api::inference::ToolSearchConfig::provider_native();
    let body = OpenAiResponsesEngine::map_request(&request);
    assert_eq!(body["tools"], json!([{ "type":"computer" }]));
    assert_eq!(body["tool_choice"], json!({"type":"computer"}));
    assert_eq!(body["parallel_tool_calls"], false);
}
#[test]
fn streamed_native_item_executes_once_only_after_complete_and_preserves_actions() {
    let mut state = ResponsesStreamState::default();
    let mut item = native_item();
    item["status"] = json!("in_progress");
    item["actions"] = json!([]);
    let added = SseEvent {
        event: None,
        data: json!({"type":"response.output_item.added","item":item}),
    };
    let events = events_from_sse_event(&added, &mut state);
    assert!(
        matches!(&events[0], InferenceEvent::ToolCallStarted(call) if call.id == "call_native" && call.name == "computer")
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, InferenceEvent::ToolCallCompleted(_)))
    );
    let done = SseEvent {
        event: None,
        data: json!({"type":"response.output_item.done","item":native_item()}),
    };
    let events = events_from_sse_event(&done, &mut state);
    let call = events
        .iter()
        .find_map(|event| match event {
            InferenceEvent::ToolCallCompleted(call) => Some(call),
            _ => None,
        })
        .unwrap();
    assert_eq!(call.id, "call_native");
    assert_eq!(call.name, "computer");
    assert_eq!(
        serde_json::from_str::<Value>(&call.arguments).unwrap()["actions"],
        native_item()["actions"]
    );
    let terminal = SseEvent {
        event: None,
        data: json!({"type":"response.completed","response":{"id":"resp_native","output":[native_item()]}}),
    };
    assert!(
        !events_from_sse_event(&terminal, &mut state)
            .iter()
            .any(|event| matches!(event, InferenceEvent::ToolCallCompleted(_)))
    );
}
#[test]
fn native_calls_and_screenshots_replay_exact_protocol_and_call_id() {
    let mut request = native_request();
    request.transcript.push(TranscriptItem::ProviderMetadata(
        json!({"output":[native_item()]}),
    ));
    request
        .transcript
        .push(TranscriptItem::ToolCall(ToolCallRecord {
            id: "call_native".into(),
            name: "computer".into(),
            arguments: json!({"actions":native_item()["actions"]}).to_string(),
        }));
    add_result(&mut request, false);
    validate_computer_request(&request, ResponsesProviderProfile::OpenAi).unwrap();
    let body = OpenAiResponsesEngine::map_request(&request);
    let input = body["input"].as_array().unwrap();
    assert_eq!(
        input
            .iter()
            .filter(|item| item["type"] == "computer_call")
            .count(),
        1
    );
    assert_eq!(
        input
            .iter()
            .find(|item| item["type"] == "computer_call")
            .unwrap(),
        &native_item()
    );
    let output = input
        .iter()
        .find(|item| item["type"] == "computer_call_output")
        .unwrap();
    assert_eq!(
        output,
        &json!({"type":"computer_call_output","call_id":"call_native","output":{
        "type":"computer_screenshot","image_url":"data:image/jpeg;base64,YWJj","detail":"original"}})
    );
    assert!(
        !input
            .iter()
            .any(|item| item["type"] == "function_call" || item["type"] == "function_call_output")
    );
}
#[test]
fn native_resume_without_provider_metadata_still_reconstructs_native_call() {
    let mut request = native_request();
    request
        .transcript
        .push(TranscriptItem::ToolCall(ToolCallRecord {
            id: "call_native".into(),
            name: "computer".into(),
            arguments: json!({"actions":native_item()["actions"]}).to_string(),
        }));
    add_result(&mut request, true);
    let input = OpenAiResponsesEngine::map_request(&request)["input"]
        .as_array()
        .unwrap()
        .clone();
    assert!(
        input
            .iter()
            .any(|item| item["type"] == "computer_call"
                && item["actions"] == native_item()["actions"])
    );
    assert!(
        input
            .iter()
            .any(|item| item["type"] == "computer_call_output")
    );
    assert!(input.iter().any(
        |item| item["role"] == "user" && item.to_string().contains("Computer execution failed")
    ));
}
#[test]
fn native_no_screenshot_and_unsupported_provider_fail_before_network() {
    let mut request = native_request();
    request.transcript.push(TranscriptItem::ProviderMetadata(
        json!({"output":[native_item()]}),
    ));
    add_result(&mut request, false);
    if let TranscriptItem::ToolResult(result) = request.transcript.last_mut().unwrap() {
        result.display_payload = None;
    }
    assert!(
        validate_computer_request(&request, ResponsesProviderProfile::OpenAi)
            .unwrap_err()
            .to_string()
            .contains("no screenshot")
    );
    request.transcript.clear();
    assert!(validate_computer_request(&request, ResponsesProviderProfile::OpenRouter).is_err());
    request.model.provider = "codex".into();
    assert!(
        validate_computer_request(&request, ResponsesProviderProfile::OpenAi)
            .unwrap_err()
            .to_string()
            .contains("Codex endpoint")
    );
}
#[test]
fn malformed_native_item_fails_protocol_instead_of_ending_the_turn_silently() {
    let mut item = native_item();
    item.as_object_mut().unwrap().remove("call_id");
    let mut state = ResponsesStreamState::default();
    let events = events_from_sse_event(
        &SseEvent {
            event: None,
            data: json!({"type":"response.output_item.done","item":item}),
        },
        &mut state,
    );
    assert!(matches!(events[0], InferenceEvent::Failed(_)));
    assert!(state.protocol_failure.is_some());
}
#[test]
fn native_screenshot_is_counted_and_never_shed_as_an_invalid_output() {
    let mut request = native_request();
    request.transcript.push(TranscriptItem::ProviderMetadata(
        json!({"output":[native_item()]}),
    ));
    add_result(&mut request, false);
    let body = OpenAiResponsesEngine::map_request(&request);
    let bytes = serde_json::to_vec(&body).unwrap().len();
    let payload = prepare_request_payload(&body, bytes).unwrap();
    assert_eq!(payload.metadata["image_count"], 1);
    assert!(prepare_request_payload(&body, bytes - 1).is_err());
}

#[tokio::test]
async fn native_websocket_stream_executes_once_and_continues_with_native_screenshot() {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let item = native_item();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
        let first: Value =
            serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(first["tools"], json!([{"type":"computer"}]));
        let frame = json!({"type":"response.output_item.done","item":item});
        socket
            .send(Message::Text(frame.to_string().into()))
            .await
            .unwrap();
        socket.send(Message::Text(json!({"type":"response.completed","response":{"id":"resp_ws_native","output":[item]}}).to_string().into())).await.unwrap();
        let next: Value =
            serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(next["previous_response_id"], "resp_ws_native");
        assert_eq!(
            next["input"],
            json!([{"type":"computer_call_output","call_id":"call_native","output":{
            "type":"computer_screenshot","image_url":"data:image/jpeg;base64,YWJj","detail":"original"}}])
        );
        socket
            .send(Message::Text(
                json!({"type":"response.completed","response":{"id":"resp_ws_done","output":[]}})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
    });
    let mut request = native_request();
    request.runtime.auto_compact_token_limit = None;
    let body = OpenAiResponsesEngine::map_request(&request);
    let first = try_websocket_stream(
        &url,
        "fixture-token",
        &[],
        "native-ws-fixture",
        &body,
        HashMap::new(),
        None,
    )
    .await
    .unwrap()
    .unwrap();
    let events: Vec<_> = first.collect().await;
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, Ok(InferenceEvent::ToolCallCompleted(_))))
            .count(),
        1
    );
    request.transcript.push(TranscriptItem::ProviderMetadata(
        json!({"output":[native_item()]}),
    ));
    add_result(&mut request, false);
    let body = OpenAiResponsesEngine::map_request(&request);
    let second = try_websocket_stream(
        &url,
        "fixture-token",
        &[],
        "native-ws-fixture",
        &body,
        HashMap::new(),
        None,
    )
    .await
    .unwrap()
    .unwrap();
    let events: Vec<_> = second.collect().await;
    assert!(events.iter().all(Result::is_ok));
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Ok(InferenceEvent::Completed(_))))
    );
    server.await.unwrap();
}
