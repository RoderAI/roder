use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[test]
fn incremental_requests_require_identical_properties_and_exact_history_prefix() {
    let previous = json!({"model":"gpt-6-sol","input":[{"type":"message","role":"user","content":"go"}],"tools":[],"instructions":"stable"});
    let output = json!({"type":"function_call","id":"fc1","call_id":"call1","name":"read_file","arguments":"{}"});
    let last = LastResponse {
        request: previous.clone(),
        output: vec![output.clone()],
        id: "response1".into(),
    };
    let mut next = previous.clone();
    next["input"].as_array_mut().unwrap().extend([
        output,
        json!({"type":"function_call_output","call_id":"call1","output":"ok"}),
    ]);
    let delta = incremental_request(&next, Some(&last));
    assert_eq!(delta["previous_response_id"], "response1");
    assert_eq!(delta["input"].as_array().unwrap().len(), 1);
    for field in [
        "model",
        "instructions",
        "tools",
        "reasoning",
        "context_management",
    ] {
        let mut changed = next.clone();
        changed[field] = json!("changed");
        assert!(
            incremental_request(&changed, Some(&last))
                .get("previous_response_id")
                .is_none(),
            "{field}"
        );
    }
    next["input"][0]["content"] = json!("steered or compacted");
    assert!(
        incremental_request(&next, Some(&last))
            .get("previous_response_id")
            .is_none()
    );
}

async fn collect(stream: InferenceEventStream) -> Vec<InferenceEvent> {
    stream.map(|event| event.unwrap()).collect().await
}

// The upstream handshake callback requires its unboxed HTTP error response.
#[allow(clippy::result_large_err)]
#[tokio::test]
async fn websocket_reuses_connection_and_sends_verified_delta() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let call = json!({"type":"function_call","id":"fc1","call_id":"call1","name":"read_file","arguments":"{}"});
    let server_call = call.clone();
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_hdr_async(
            socket,
            |request: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
                assert_eq!(request.headers()["authorization"], "Bearer fixture-key");
                assert_eq!(
                    request.headers()["openai-beta"],
                    "responses_websockets=2026-02-06"
                );
                assert_eq!(request.headers()["session_id"], "ws-thread");
                Ok(response)
            },
        )
        .await
        .unwrap();
        let first: Value =
            serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(first["type"], "response.create");
        assert!(first.get("previous_response_id").is_none());
        socket
            .send(Message::Text(
                json!({"type":"response.output_item.done","item":server_call})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        socket.send(Message::Text(json!({"type":"response.completed","response":{"id":"response1","output":[server_call]}}).to_string().into())).await.unwrap();
        let second: Value =
            serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(second["previous_response_id"], "response1");
        assert_eq!(second["input"].as_array().unwrap().len(), 1);
        assert_eq!(second["input"][0]["type"], "function_call_output");
        socket
            .send(Message::Text(
                json!({"type":"response.completed","response":{"id":"response2","output":[]}})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
    });
    let body = json!({"model":"gpt-6-sol","stream":true,"input":[],"tools":[]});
    let events = collect(
        try_websocket_stream(
            &base,
            "fixture-key",
            &[],
            "ws-thread",
            &body,
            HashMap::new(),
            None,
        )
        .await
        .unwrap()
        .unwrap(),
    )
    .await;
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, InferenceEvent::ToolCallCompleted(_)))
            .count(),
        1
    );
    let mut next = body;
    next["input"].as_array_mut().unwrap().extend([
        call,
        json!({"type":"function_call_output","call_id":"call1","output":"ok"}),
    ]);
    let events = collect(
        try_websocket_stream(
            &base,
            "fixture-key",
            &[],
            "ws-thread",
            &next,
            HashMap::new(),
            None,
        )
        .await
        .unwrap()
        .unwrap(),
    )
    .await;
    assert!(events.iter().any(
        |event| matches!(event,InferenceEvent::ProviderMetadata(metadata)
        if metadata["connection_reused"] == true && metadata["incremental"] == true)
    ));
    server.await.unwrap();
}

#[tokio::test]
async fn unsupported_websocket_endpoint_falls_back_to_http_without_sampling_twice() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut first, _) = listener.accept().await.unwrap();
        let mut bytes = [0; 4096];
        let count = first.read(&mut bytes).await.unwrap();
        assert!(
            std::str::from_utf8(&bytes[..count])
                .unwrap()
                .starts_with("GET /responses")
        );
        first
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        drop(first);
        let (mut second, _) = listener.accept().await.unwrap();
        let count = second.read(&mut bytes).await.unwrap();
        assert!(
            std::str::from_utf8(&bytes[..count])
                .unwrap()
                .starts_with("POST /responses")
        );
        second
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
    });
    let body = json!({"model":"gpt-6-sol","input":[]});
    assert!(
        try_websocket_stream(
            &base,
            "fixture-key",
            &[],
            "fallback-thread",
            &body,
            HashMap::new(),
            None
        )
        .await
        .unwrap()
        .is_none()
    );
    send_responses_request(&base, "fixture-key", &[], None, &body, None)
        .await
        .unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn dropping_unfinished_websocket_invalidates_connection_and_continuation() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
        socket.next().await.unwrap().unwrap();
        socket
            .send(Message::Text(
                json!({"type":"response.output_text.delta","delta":"unfinished"})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap();
        let (next, _) = listener.accept().await.unwrap();
        let mut next = tokio_tungstenite::accept_async(next).await.unwrap();
        let request: Value =
            serde_json::from_str(next.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert!(request.get("previous_response_id").is_none());
        next.send(Message::Text(
            json!({"type":"response.completed","response":{"id":"fresh","output":[]}})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    });
    let body = json!({"model":"gpt-6-sol","input":[]});
    let mut stream = try_websocket_stream(
        &base,
        "fixture-key",
        &[],
        "drop-thread",
        &body,
        HashMap::new(),
        None,
    )
    .await
    .unwrap()
    .unwrap();
    while !matches!(
        stream.next().await.unwrap().unwrap(),
        InferenceEvent::MessageDelta(_)
    ) {}
    drop(stream);
    collect(
        try_websocket_stream(
            &base,
            "fixture-key",
            &[],
            "drop-thread",
            &body,
            HashMap::new(),
            None,
        )
        .await
        .unwrap()
        .unwrap(),
    )
    .await;
    server.await.unwrap();
}

#[tokio::test]
async fn websocket_client_search_returns_loadable_schema_then_completes_once() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
        socket.next().await.unwrap().unwrap();
        let search = json!({"type":"tool_search_call","id":"search1","call_id":"search-call1","execution":"client","arguments":{"query":"echo","limit":1},"status":"completed"});
        socket.send(Message::Text(json!({"type":"response.completed","response":{"id":"search-response","output":[search]}}).to_string().into())).await.unwrap();
        let next: Value =
            serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(next["previous_response_id"], "search-response");
        assert_eq!(next["input"].as_array().unwrap().len(), 1);
        assert_eq!(next["input"][0]["call_id"], "search-call1");
        assert_eq!(next["input"][0]["tools"][0]["parameters"]["type"], "object");
        assert!(next["input"][0]["tools"][0].get("defer_loading").is_none());
        socket
            .send(Message::Text(
                json!({"type":"response.completed","response":{"id":"done","output":[]}})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
    });
    let mut request = crate::provider::tests::request();
    request.runtime.tool_search = roder_api::inference::ToolSearchConfig::provider_native();
    let (body, map) =
        OpenAiResponsesEngine::map_request_with_options(&request, RequestMappingOptions::default());
    let ctx = ClientToolSearchContext {
        base_url: base.clone(),
        api_key: "fixture-key".into(),
        headers: vec![],
        grok_conversation_id: None,
        body: body.clone(),
        policy: None,
        definitions: client_search_definitions(&body, &map),
        catalog: roder_api::tool_search_catalog::ToolSearchCatalog::build(
            &request.tools,
            &request.runtime.tool_search,
        ),
    };
    let events = collect(
        try_websocket_stream(
            &base,
            "fixture-key",
            &[],
            "search-thread",
            &body,
            map.api_name_to_tool_name,
            Some(ctx),
        )
        .await
        .unwrap()
        .unwrap(),
    )
    .await;
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, InferenceEvent::Completed(_)))
            .count(),
        1
    );
    assert!(events.iter().any(|event|matches!(event,InferenceEvent::OutputItemCompleted(item) if item["type"]=="tool_search_output")));
    server.await.unwrap();
}

#[tokio::test]
async fn missing_previous_response_repairs_once_with_full_request() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
        socket.next().await.unwrap().unwrap();
        socket
            .send(Message::Text(
                json!({"type":"response.completed","response":{"id":"gone","output":[]}})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        let delta: Value =
            serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(delta["previous_response_id"], "gone");
        socket.send(Message::Text(json!({"type":"error","error":{"code":"previous_response_not_found","message":"expired"}}).to_string().into())).await.unwrap();
        let repaired: Value =
            serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert!(repaired.get("previous_response_id").is_none());
        assert_eq!(repaired["input"].as_array().unwrap().len(), 2);
        socket
            .send(Message::Text(
                json!({"type":"response.completed","response":{"id":"repaired","output":[]}})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
    });
    let body =
        json!({"model":"gpt-6-sol","input":[{"type":"message","role":"user","content":"first"}]});
    collect(
        try_websocket_stream(
            &base,
            "fixture-key",
            &[],
            "repair-thread",
            &body,
            HashMap::new(),
            None,
        )
        .await
        .unwrap()
        .unwrap(),
    )
    .await;
    let mut next = body;
    next["input"]
        .as_array_mut()
        .unwrap()
        .push(json!({"type":"message","role":"user","content":"second"}));
    collect(
        try_websocket_stream(
            &base,
            "fixture-key",
            &[],
            "repair-thread",
            &next,
            HashMap::new(),
            None,
        )
        .await
        .unwrap()
        .unwrap(),
    )
    .await;
    server.await.unwrap();
}

#[test]
fn rotated_credentials_or_account_headers_never_reuse_a_session() {
    let key = SessionKey {
        base_url: "http://identity-fixture".into(),
        thread_id: "same-thread".into(),
        token: "first-token".into(),
        headers: vec![("chatgpt-account-id".into(), "account-one".into())],
    };
    let first = session(key.clone()).unwrap();
    assert!(Arc::ptr_eq(&first, &session(key.clone()).unwrap()));
    let mut rotated = key.clone();
    rotated.token = "rotated-token".into();
    assert!(!Arc::ptr_eq(&first, &session(rotated).unwrap()));
    let mut account = key;
    account.headers[0].1 = "account-two".into();
    assert!(!Arc::ptr_eq(&first, &session(account).unwrap()));
}

#[test]
fn native_computer_continuation_uses_previous_response_id_and_only_new_screenshot() {
    let previous = json!({"model":"gpt-6-sol","tools":[{"type":"computer"}],"input":[{"role":"user","content":"go"}]});
    let output = json!({"type":"computer_call","id":"cu_1","call_id":"native_1","actions":[{"type":"screenshot"}],"status":"completed"});
    let last = LastResponse {
        request: previous.clone(),
        output: vec![output.clone()],
        id: "resp_native".into(),
    };
    let mut next = previous;
    let screenshot = json!({"type":"computer_call_output","call_id":"native_1","output":{
        "type":"computer_screenshot","image_url":"data:image/png;base64,YWJj","detail":"original"}});
    next["input"]
        .as_array_mut()
        .unwrap()
        .extend([output, screenshot.clone()]);
    let delta = incremental_request(&next, Some(&last));
    assert_eq!(delta["previous_response_id"], "resp_native");
    assert_eq!(delta["input"], json!([screenshot]));
    assert_eq!(delta["tools"], json!([{"type":"computer"}]));
}
