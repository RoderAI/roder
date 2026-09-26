use super::*;
use futures::StreamExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

// Contract assertions deliberately fail on the audited source snapshot.
async fn chunk_server(parts: Vec<Vec<u8>>, delay_between: Duration, hold_open: Duration) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = vec![0; 65536];
        let _ = socket.read(&mut request).await.unwrap();
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").await.unwrap();
        for part in parts {
            socket
                .write_all(format!("{:x}\r\n", part.len()).as_bytes())
                .await
                .unwrap();
            socket.write_all(&part).await.unwrap();
            socket.write_all(b"\r\n").await.unwrap();
            socket.flush().await.unwrap();
            tokio::time::sleep(delay_between).await;
        }
        tokio::time::sleep(hold_open).await;
        let _ = socket.write_all(b"0\r\n\r\n").await;
    });
    format!("http://{addr}")
}

async fn open_stream(base_url: &str) -> InferenceEventStream {
    let response = send_responses_request(base_url, "fixture-key", &[], None, &json!({}), None)
        .await
        .unwrap();
    stream_responses_sse_with_client_tool_search(
        response.response,
        HashMap::new(),
        response.retry_events,
        response.idle_timeout,
        None,
    )
}

#[tokio::test]
async fn audit_split_utf8_preserves_text() {
    let payload = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"café\"}\n\ndata: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"output\":[]}}\n\n".as_bytes();
    let split = payload.iter().position(|b| *b == 0xc3).unwrap() + 1;
    let base_url = chunk_server(
        vec![payload[..split].to_vec(), payload[split..].to_vec()],
        Duration::from_millis(30),
        Duration::ZERO,
    )
    .await;
    let mut stream = open_stream(&base_url).await;
    let mut text = String::new();
    while let Some(event) = stream.next().await {
        if let InferenceEvent::MessageDelta(delta) = event.unwrap() {
            text.push_str(&delta.text);
        }
    }
    assert_eq!(text, "café");
}

#[tokio::test]
async fn audit_completed_ends_without_waiting_for_eof() {
    let payload = b"data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"output\":[]}}\n\n".to_vec();
    let base_url = chunk_server(vec![payload], Duration::ZERO, Duration::from_millis(500)).await;
    let mut stream = open_stream(&base_url).await;
    while !matches!(
        stream.next().await.unwrap().unwrap(),
        InferenceEvent::Completed(_)
    ) {}
    assert!(
        matches!(
            tokio::time::timeout(Duration::from_millis(100), stream.next()).await,
            Ok(None)
        ),
        "response.completed must end the stream even if HTTP remains open"
    );
}

#[test]
fn audit_codex_client_search_shape_is_recognized() {
    // Exact fields from codex-rs/protocol/src/models.rs::tool_search_call_roundtrips.
    let item = json!({"type":"tool_search_call","call_id":"search-1","execution":"client","arguments":{"query":"calendar create","limit":1}});
    assert!(
        is_client_executed_tool_search(&item),
        "client-executed Codex search was not recognized: {item}"
    );
}

#[tokio::test]
async fn audit_search_limit_returns_error_instead_of_silent_eof() {
    let bodies = Arc::new(std::sync::Mutex::new(Vec::new()));
    const SEARCH: &str = "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_search\",\"status\":\"completed\",\"output\":[{\"id\":\"ts_search\",\"type\":\"tool_search_call\",\"query\":\"read files\"}]}}\n\n";
    const FINAL: &str = "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_final\",\"status\":\"completed\",\"output\":[]}}\n\n";
    let mut responses = vec![("/responses", 200, SEARCH); MAX_CLIENT_TOOL_SEARCH_ROUNDS + 1];
    responses.push(("/responses", 200, FINAL));
    let base_url = spawn_recording_server(responses, Arc::clone(&bodies)).await;
    let body = json!({"model":"gpt-5.5","input":[]});
    let response = send_responses_request(&base_url, "fixture-key", &[], None, &body, None)
        .await
        .unwrap();
    let catalog = roder_api::tool_search_catalog::ToolSearchCatalog::build(
        &[],
        &roder_api::inference::ToolSearchConfig::default(),
    );
    let mut stream = stream_responses_sse_with_client_tool_search(
        response.response,
        HashMap::new(),
        response.retry_events,
        response.idle_timeout,
        Some(ClientToolSearchContext {
            base_url,
            api_key: "fixture-key".into(),
            headers: vec![],
            grok_conversation_id: None,
            body,
            policy: None,
            catalog,
        }),
    );
    let mut completed = 0;
    let mut errors = 0;
    while let Some(event) = stream.next().await {
        match event {
            Ok(InferenceEvent::Completed(_)) => completed += 1,
            Err(_) => errors += 1,
            _ => {}
        }
    }
    let request_count = bodies.lock().unwrap().len();
    assert!(
        completed == 1 || errors == 1,
        "silent EOF: {request_count} requests, {completed} Completed events, {errors} errors"
    );
}

async fn spawn_recording_server(
    routes: Vec<(&'static str, u16, &'static str)>,
    bodies: Arc<std::sync::Mutex<Vec<String>>>,
) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        for (_, status, body) in routes {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut data = Vec::new();
            let header_end = loop {
                let mut buf = [0; 4096];
                let n = socket.read(&mut buf).await.unwrap();
                assert!(n > 0);
                data.extend_from_slice(&buf[..n]);
                if let Some(pos) = data.windows(4).position(|x| x == b"\r\n\r\n") {
                    break pos + 4;
                }
            };
            let headers = String::from_utf8_lossy(&data[..header_end]);
            let length: usize = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|x| x.trim().parse().unwrap())
                })
                .unwrap_or(0);
            while data.len() < header_end + length {
                let mut buf = [0; 4096];
                let n = socket.read(&mut buf).await.unwrap();
                assert!(n > 0);
                data.extend_from_slice(&buf[..n]);
            }
            bodies
                .lock()
                .unwrap()
                .push(String::from_utf8_lossy(&data[header_end..]).to_string());
            let response = format!(
                "HTTP/1.1 {status} OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        }
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn audit_http_retry_honors_retry_after() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut first, _) = listener.accept().await.unwrap();
        let mut buf = [0; 4096];
        first.read(&mut buf).await.unwrap();
        let started = std::time::Instant::now();
        first.write_all(b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
        drop(first);
        let (mut second, _) = listener.accept().await.unwrap();
        let elapsed = started.elapsed();
        second.read(&mut buf).await.unwrap();
        second
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        elapsed
    });
    let policy = ReliabilityRequestPolicy {
        provider_retry_max_attempts: 2,
        provider_retry_initial_backoff_ms: 0,
        ..Default::default()
    };
    let _ = send_responses_request(
        &format!("http://{address}"),
        "fixture-key",
        &[],
        None,
        &json!({}),
        Some(&policy),
    )
    .await
    .unwrap();
    let elapsed = task.await.unwrap();
    assert!(
        elapsed >= Duration::from_millis(900),
        "Retry-After: 1 ignored; second request after {elapsed:?}"
    );
}
