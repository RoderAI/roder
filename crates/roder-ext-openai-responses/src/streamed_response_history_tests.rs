use super::*;

fn output_items() -> Vec<Value> {
    vec![
        json!({
            "type": "reasoning", "id": "rs_history",
            "encrypted_content": "opaque-fixture", "summary": []
        }),
        json!({
            "type": "function_call", "id": "fc_history", "call_id": "call_history",
            "name": "read_file", "arguments": "{}", "status": "completed"
        }),
    ]
}

fn completed_item(item: &Value) -> SseEvent {
    SseEvent {
        event: None,
        data: json!({"type": "response.output_item.done", "item": item}),
    }
}

#[test]
fn streamed_history_survives_empty_or_missing_terminal_output() {
    let output = output_items();
    for terminal in [None, Some(json!([])), Some(json!(output))] {
        let mut state = ResponsesStreamState::default();
        for item in &output {
            events_from_sse_event(&completed_item(item), &mut state);
        }
        let mut response = json!({"id": "response_history", "status": "completed"});
        if let Some(terminal) = terminal {
            response["output"] = terminal;
        }
        let events = events_from_sse_event(
            &SseEvent {
                event: None,
                data: json!({"type": "response.completed", "response": response}),
            },
            &mut state,
        );
        assert_eq!(state.response_output, output);
        assert!(events.iter().any(|event| {
            matches!(event, InferenceEvent::ProviderMetadata(metadata)
                if metadata["output"] == json!(output))
        }));
    }
}

#[test]
fn terminal_output_is_authoritative_without_duplicate_streamed_items() {
    let output = output_items();
    let mut state = ResponsesStreamState::default();
    for item in &output {
        events_from_sse_event(&completed_item(item), &mut state);
        events_from_sse_event(&completed_item(item), &mut state);
    }
    let mut terminal = output.clone();
    terminal.push(json!({
        "id": "msg_terminal", "type": "message", "role": "assistant",
        "phase": "commentary", "content": [{"type": "output_text", "text": "Checking."}]
    }));
    let events = events_from_sse_event(
        &SseEvent {
            event: None,
            data: json!({
                "type": "response.completed",
                "response": {"id": "response_history", "output": terminal}
            }),
        },
        &mut state,
    );
    assert_eq!(state.response_output, terminal);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, InferenceEvent::OutputItemCompleted(_)))
            .count(),
        1
    );
}

#[tokio::test]
async fn websocket_continuation_does_not_resend_streamed_reasoning_or_calls() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let output = output_items();
    let server_output = output.clone();
    let result = json!({"type": "function_call_output", "call_id": "call_history", "output": "ok"});
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
        socket.next().await.unwrap().unwrap();
        for item in &server_output {
            socket
                .send(Message::Text(completed_item(item).data.to_string().into()))
                .await
                .unwrap();
        }
        socket
            .send(Message::Text(
                json!({
                    "type": "response.completed",
                    "response": {"id": "response_history", "output": []}
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        let request: Value =
            serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        socket
            .send(Message::Text(
                json!({
                    "type": "response.completed",
                    "response": {"id": "response_next", "output": []}
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        request
    });
    let body = json!({
        "model": "gpt-6-sol", "stream": true, "tools": [],
        "input": [{"type": "message", "role": "user", "content": "Inspect the file."}]
    });
    for request in [body.clone(), {
        let mut next = body;
        let input = next["input"].as_array_mut().unwrap();
        input.extend(output);
        input.push(result.clone());
        next
    }] {
        let stream = try_websocket_stream(
            &base,
            "fixture-key",
            &[],
            "streamed-history",
            &request,
            HashMap::new(),
            None,
        )
        .await
        .unwrap()
        .unwrap();
        tokio::time::timeout(
            Duration::from_secs(5),
            stream.map(|event| event.unwrap()).collect::<Vec<_>>(),
        )
        .await
        .unwrap();
    }
    let request = server.await.unwrap();
    assert_eq!(request["previous_response_id"], "response_history");
    assert_eq!(
        request["input"],
        json!([result]),
        "previous_response_id already includes the streamed reasoning and call"
    );
}

#[test]
fn changed_message_replay_shape_uses_full_history_without_previous_response_id() {
    let body = json!({
        "model": "gpt-6-sol",
        "input": [{"type": "message", "role": "user", "content": "Inspect the file."}]
    });
    let message = json!({
        "id": "msg_history", "type": "message", "role": "assistant",
        "phase": "commentary", "content": [{"type": "output_text", "text": "Checking."}]
    });
    let mut state = ResponsesStreamState::default();
    events_from_sse_event(&completed_item(&message), &mut state);
    events_from_sse_event(
        &SseEvent {
            event: None,
            data: json!({
                "type": "response.completed",
                "response": {"id": "response_history", "output": []}
            }),
        },
        &mut state,
    );
    let last = LastResponse {
        request: body.clone(),
        output: state.response_output,
        id: "response_history".into(),
    };
    let mut replayed_message = message;
    replayed_message.as_object_mut().unwrap().remove("id");
    let mut next = body;
    next["input"].as_array_mut().unwrap().push(replayed_message);
    let request = incremental_request(&next, Some(&last));
    assert!(
        request.get("previous_response_id").is_none(),
        "a differently serialized assistant message must not be appended to its own response"
    );
    assert_eq!(request["input"], next["input"]);
}
