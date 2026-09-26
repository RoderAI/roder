use super::*;

#[test]
fn patch_grammar_is_identical_for_openai_and_codex_coding_models() {
    for provider in [PROVIDER_OPENAI, PROVIDER_CODEX] {
        for model in [
            "gpt-5",
            "gpt-5.4",
            "gpt-5.5",
            "gpt-5.6-sol",
            "gpt-6-astra",
            "gpt-6-sol",
            "gpt-6-luna",
        ] {
            let mut request = super::tests::request();
            request.model.provider = provider.to_string();
            request.model.model = model.to_string();
            request.tools = vec![roder_api::tools::ToolSpec {
                name: "apply_patch".into(),
                description: "edit".into(),
                parameters: json!({"type":"object","properties":{"patch":{"type":"string"}},"required":["patch"],"additionalProperties":false}),
            }];
            let body = OpenAiResponsesEngine::map_request(&request);
            let tool = &body["tools"][0];
            assert_eq!(tool["type"], "custom", "{provider}/{model}");
            assert_eq!(
                tool["format"],
                json!({"type":"grammar", "syntax":"lark", "definition":include_str!("../assets/apply_patch.lark")})
            );
            assert!(tool.get("parameters").is_none());
        }
    }
}

fn event(data: Value) -> SseEvent {
    SseEvent { event: None, data }
}

#[test]
fn streamed_custom_patch_completes_once_after_output_item_done() {
    let mut state = ResponsesStreamState::default();
    let item = json!({"type":"custom_tool_call", "id":"patch_item", "call_id":"patch_call", "name":"apply_patch"});
    assert!(
        matches!(&events_from_sse_event(&event(json!({"type":"response.output_item.added","item":item})), &mut state)[0], InferenceEvent::ToolCallStarted(call) if call.id == "patch_call")
    );
    for delta in [
        "*** Begin Patch\n*** Add File: café.txt\n",
        "+hello\n*** End Patch",
    ] {
        let events = events_from_sse_event(
            &event(
                json!({"type":"response.custom_tool_call_input.delta", "item_id":"patch_item", "delta":delta}),
            ),
            &mut state,
        );
        assert!(
            matches!(&events[0], InferenceEvent::ToolCallDelta(call) if call.id == "patch_call" && call.arguments_delta == delta)
        );
    }
    let patch = "*** Begin Patch\n*** Add File: café.txt\n+hello\n*** End Patch";
    assert!(events_from_sse_event(&event(json!({"type":"response.custom_tool_call_input.done", "item_id":"patch_item", "input":patch})), &mut state).is_empty());
    let events = events_from_sse_event(
        &event(json!({"type":"response.output_item.done", "item":item})),
        &mut state,
    );
    assert!(
        matches!(&events[0], InferenceEvent::ToolCallCompleted(call) if call.id == "patch_call" && serde_json::from_str::<Value>(&call.arguments).unwrap() == json!({"patch":patch}))
    );
    let mut completed_item = item;
    completed_item["input"] = json!(patch);
    let events = events_from_sse_event(
        &event(
            json!({"type":"response.completed", "response":{"id":"response", "output":[completed_item]}}),
        ),
        &mut state,
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, InferenceEvent::ToolCallCompleted(_)))
    );
}

#[test]
fn sse_frame_decoder_handles_mixed_line_endings_and_invalid_utf8() {
    for payload in [
        b"data: {}\r\n\r\n".as_slice(),
        b"data: {}\r\rX",
        b"data: {}\n\r\n",
        b"data: {}\r\n\n",
    ] {
        let (frame, _) = take_sse_frame(payload).unwrap();
        assert_eq!(
            parse_sse_frame(std::str::from_utf8(&frame).unwrap())
                .unwrap()
                .unwrap()
                .data,
            json!({})
        );
    }
    let (frame, _) = take_sse_frame(b"data: \xff\n\n").unwrap();
    assert!(std::str::from_utf8(&frame).is_err());
}

#[test]
fn tool_search_exchanges_survive_transcript_replay() {
    use roder_api::transcript::TranscriptItem;
    let mut request = super::tests::request();
    request.transcript = vec![TranscriptItem::ProviderMetadata(json!({"output":[
        {"type":"tool_search_call","call_id":"search", "execution":"client", "arguments":{"query":"read"}},
        {"type":"tool_search_output", "call_id":"search", "execution":"client", "status":"completed", "tools":[{"type":"function","name":"read_file","parameters":{"type":"object"}}]}
    ]}))];
    let body = OpenAiResponsesEngine::map_request(&request);
    assert_eq!(body["input"][0]["type"], "tool_search_call");
    assert_eq!(body["input"][1]["tools"][0]["name"], "read_file");
}

#[test]
fn retry_after_accepts_seconds_and_dates_and_rejects_invalid_values() {
    let before = tokio::time::Instant::now();
    assert!(retry_after_deadline(" 2 ").unwrap() >= before + Duration::from_secs(2));
    let date = httpdate::fmt_http_date(SystemTime::now() + Duration::from_secs(5));
    assert!(retry_after_deadline(&date).unwrap() >= before + Duration::from_secs(3));
    assert!(retry_after_deadline("-1").is_none());
    assert!(retry_after_deadline("1.5").is_none());
}

#[test]
fn incremental_and_final_output_replay_once_without_orphan_searches() {
    use roder_api::transcript::{
        ReasoningSummary, ToolCallRecord, ToolResultRecord, TranscriptItem,
    };
    let mut request = super::tests::request();
    let reasoning = json!({"id":"reason1","type":"reasoning","encrypted_content":"opaque","summary":[{"type":"summary_text","text":"think"}]});
    let call = json!({"id":"fc1","type":"custom_tool_call","call_id":"patch1","name":"apply_patch","input":"patch"});
    let search = json!({"type":"tool_search_call","call_id":"search1","execution":"client","arguments":{"query":"edit"}});
    let result =
        json!({"type":"tool_search_output","call_id":"search1","execution":"client","tools":[]});
    request.transcript = vec![
        TranscriptItem::ReasoningSummary(ReasoningSummary {
            text: "think".into(),
        }),
        TranscriptItem::ProviderMetadata(
            json!({"output":[reasoning.clone(), call.clone(), search.clone()]}),
        ),
        TranscriptItem::ProviderMetadata(json!({"output":[reasoning,call,search,result,
            {"type":"tool_search_output","call_id":"orphan","tools":[]}]})),
        TranscriptItem::ToolCall(ToolCallRecord {
            id: "patch1".into(),
            name: "apply_patch".into(),
            arguments: "{}".into(),
        }),
        TranscriptItem::ToolResult(ToolResultRecord {
            id: "patch1".into(),
            name: Some("apply_patch".into()),
            result: "ok".into(),
            display_payload: None,
            is_error: false,
        }),
    ];
    let body = OpenAiResponsesEngine::map_request(&request);
    let items = body["input"].as_array().unwrap();
    for kind in [
        "reasoning",
        "custom_tool_call",
        "custom_tool_call_output",
        "tool_search_call",
        "tool_search_output",
    ] {
        assert_eq!(
            items.iter().filter(|item| item["type"] == kind).count(),
            1,
            "{kind}: {items:?}"
        );
    }
    assert!(!body.to_string().contains("orphan"));
}

#[test]
fn completed_output_is_durable_once_and_arguments_done_never_dispatches() {
    let mut state = ResponsesStreamState::default();
    let item = json!({"id":"reason1","type":"reasoning","encrypted_content":"opaque","summary":[]});
    let first = events_from_sse_event(
        &event(json!({"type":"response.output_item.done","item":item})),
        &mut state,
    );
    assert_eq!(
        first,
        vec![InferenceEvent::OutputItemCompleted(item.clone())]
    );
    let final_events = events_from_sse_event(
        &event(json!({"type":"response.completed","response":{"output":[item]}})),
        &mut state,
    );
    assert!(
        !final_events
            .iter()
            .any(|event| matches!(event, InferenceEvent::OutputItemCompleted(_)))
    );
}

#[test]
fn reusing_a_tool_call_id_with_different_arguments_fails_closed() {
    let mut state = ResponsesStreamState::default();
    let call = ToolCallCompleted {
        id: "once".into(),
        name: "apply_patch".into(),
        arguments: "{\"patch\":\"first\"}".into(),
    };
    assert!(matches!(
        emit_tool_call_once(call.clone(), &mut state),
        Some(InferenceEvent::ToolCallCompleted(_))
    ));
    assert!(emit_tool_call_once(call.clone(), &mut state).is_none());
    let mut changed = call;
    changed.arguments = "{\"patch\":\"different\"}".into();
    assert!(matches!(
        emit_tool_call_once(changed, &mut state),
        Some(InferenceEvent::Failed(_))
    ));
    assert!(state.protocol_failure.is_some());
}

#[test]
fn eager_call_and_completed_message_replay_original_ids_once() {
    use roder_api::transcript::{
        AssistantMessage, ToolCallRecord, ToolResultRecord, TranscriptItem,
    };
    let mut request = crate::provider::tests::request();
    let message = json!({"id":"message1","type":"message","role":"assistant","phase":"commentary","content":[{"type":"output_text","text":"checking"}],"status":"completed"});
    let call = json!({"id":"fc1","type":"function_call","call_id":"read1","name":"read_file","arguments":"{}","status":"completed"});
    request.transcript = vec![
        TranscriptItem::AssistantMessage(AssistantMessage {
            text: "checking".into(),
            phase: Some("commentary".into()),
        }),
        TranscriptItem::ProviderMetadata(json!({"output":[message]})),
        TranscriptItem::ToolCall(ToolCallRecord {
            id: "read1".into(),
            name: "read_file".into(),
            arguments: "{}".into(),
        }),
        TranscriptItem::ProviderMetadata(json!({"output":[call]})),
        TranscriptItem::ToolResult(ToolResultRecord {
            id: "read1".into(),
            name: Some("read_file".into()),
            result: "ok".into(),
            display_payload: None,
            is_error: false,
        }),
        TranscriptItem::ProviderMetadata(json!({"output":[message,call]})),
    ];
    let body = OpenAiResponsesEngine::map_request(&request);
    let input = body["input"].as_array().unwrap();
    assert_eq!(input.len(), 3);
    assert_eq!(input[0], message);
    assert_eq!(input[1], call);
    assert_eq!(input[2]["type"], "function_call_output");
}
