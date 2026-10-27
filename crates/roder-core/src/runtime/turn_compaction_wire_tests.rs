use super::*;
use roder_ext_openai_responses::OpenAiResponsesEngine;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[test]
fn commit_suffix_expands_once_and_nested_or_invalid_suffix_is_quarantined() {
    let tail = TranscriptItem::UserMessage(UserMessage::text("tail"));
    let boundary = json!({"output":[opaque()],COMMIT_FIELD:{"version":1,"retained_transcript":[tail.clone()]}});
    let items = vec![
        TranscriptItem::UserMessage(UserMessage::text("before")),
        TranscriptItem::ProviderMetadata(boundary.clone()),
    ];
    let once = crate::compaction::trim_to_last_compaction_boundary(items);
    assert_eq!(once.len(), 2);
    assert_eq!(
        crate::compaction::trim_to_last_compaction_boundary(once.clone()),
        once
    );
    for retained in [
        json!("invalid"),
        json!([TranscriptItem::ProviderMetadata(boundary)]),
    ] {
        let items = vec![
            tail.clone(),
            TranscriptItem::ProviderMetadata(
                json!({"output":[opaque()],COMMIT_FIELD:{"version":1,"retained_transcript":retained}}),
            ),
        ];
        assert_eq!(
            crate::compaction::trim_to_last_compaction_boundary(items),
            vec![tail.clone()]
        );
    }
}

async fn request(socket: &mut TcpStream) -> Value {
    let mut bytes = vec![];
    let mut buf = [0; 4096];
    let header_end = loop {
        let n = socket.read(&mut buf).await.unwrap();
        assert!(n > 0);
        bytes.extend_from_slice(&buf[..n]);
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            break end + 4;
        }
    };
    let length: usize = String::from_utf8_lossy(&bytes[..header_end])
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .and_then(|value| value.trim().parse().ok())
        })
        .unwrap();
    while bytes.len() < header_end + length {
        let n = socket.read(&mut buf).await.unwrap();
        assert!(n > 0);
        bytes.extend_from_slice(&buf[..n]);
    }
    let body: Value = serde_json::from_slice(&bytes[header_end..]).unwrap();
    assert!(!body.to_string().contains(COMMIT_FIELD));
    body
}
fn event(value: Value) -> String {
    format!("data: {value}\n\n")
}
async fn reply(socket: &mut TcpStream, sse: &str) {
    socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{sse}",sse.len()).as_bytes()).await.unwrap();
}
fn final_event() -> String {
    event(
        json!({"type":"response.completed","response":{"id":"final","output":[],"usage":{"input_tokens":2,"output_tokens":1,"total_tokens":3}}}),
    )
}
fn assert_wire_suffix(body: &Value) {
    let input = body["input"].as_array().unwrap();
    for (kind, id) in [
        ("compaction", "opaque"),
        ("message", "message"),
        ("function_call", "fc_probe"),
    ] {
        assert_eq!(
            input
                .iter()
                .filter(|item| item["type"] == kind && item["id"] == id)
                .count(),
            1,
            "{body}"
        );
    }
    assert_eq!(input.iter().filter(|item|item["type"]=="function_call_output"&&item["call_id"]=="probe-call").count(),1,"{body}");
    assert_eq!(
        input
            .iter()
            .filter(
                |item| item["type"] == "message" && item.to_string().contains("retained response")
            )
            .count(),
        1,
        "{body}"
    );
    assert!(!body.to_string().contains("original goal evidence"));
    assert!(!body.to_string().contains("original tool evidence"));
    let call = input
        .iter()
        .position(|item| item["type"] == "function_call")
        .unwrap();
    let result = input
        .iter()
        .position(|item| item["type"] == "function_call_output")
        .unwrap();
    assert!(call < result, "{body}");
}

#[tokio::test]
async fn ordinary_sse_compaction_replays_message_and_eager_call_result_once_after_restart() {
    for explicit_window in [false, true] {
        successful_wire_replay(explicit_window).await;
    }
}

async fn successful_wire_replay(explicit_window: bool) {
    let directory = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let ran = Arc::new(Notify::new());
    let server_ran = ran.clone();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let first = request(&mut socket).await;
        assert!(first.get("context_management").is_some(), "{first}");
        let prefix = [
            event(json!({"type":"response.output_item.done","item":opaque()})),
            event(json!({"type":"response.output_item.done","item":raw_message()})),
            event(json!({"type":"response.output_item.done","item":raw_call()})),
        ]
        .concat();
        let mut response = json!({
            "id":"compacted","output":[opaque(),raw_message(),raw_call()],
            "usage":{"input_tokens":8,"output_tokens":2,"total_tokens":10}
        });
        if explicit_window {
            response["compacted_input"] = json!([opaque(), raw_message(), raw_call()]);
        }
        let terminal = event(json!({"type":"response.completed","response":response}));
        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{prefix}",prefix.len()+terminal.len()).as_bytes()).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), server_ran.notified())
            .await
            .unwrap();
        socket.write_all(terminal.as_bytes()).await.unwrap();
        for _ in 0..2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            assert_wire_suffix(&request(&mut socket).await);
            reply(&mut socket, &final_event()).await;
        }
    });
    let engine = Arc::new(OpenAiResponsesEngine::new_with_config(
        Some("fixture".into()),
        "openai",
        base,
        vec![],
    ));
    let executions = Arc::new(AtomicUsize::new(0));
    let live = runtime(
        directory.path(),
        engine.clone(),
        ran.clone(),
        executions.clone(),
    );
    let thread = live.create_thread(None).await.unwrap().thread_id;
    seed(&live, &thread).await;
    let events = run(&live, directory.path(), &thread, None).await;
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind == "context.compaction_recorded")
            .count(),
        1
    );
    let progress: Vec<_> = events
        .iter()
        .filter_map(|event| {
            if let RoderEvent::InferenceEventReceived(received) = &event.event
                && let InferenceEvent::Compaction(progress) = &received.event
            {
                Some(progress.status.as_str())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(progress, vec!["pending_commit", "completed"]);
    let commit = events
        .iter()
        .position(|event| event.kind == "context.compaction_recorded")
        .unwrap();
    let completed = events
        .iter()
        .position(|event| {
            matches!(
                &event.event,RoderEvent::InferenceEventReceived(received)
                    if matches!(&received.event,InferenceEvent::Completed(_))
            )
        })
        .unwrap();
    assert!(commit < completed);
    let snapshot = live.load_thread(&thread).await.unwrap().unwrap();
    assert_eq!(
        snapshot
            .turns
            .iter()
            .flat_map(|turn| &turn.items)
            .filter(|item| crate::compaction::is_compaction_boundary(item))
            .count(),
        1
    );
    assert_eq!(executions.load(Ordering::SeqCst), 1);
    drop(live);
    let cold = runtime(directory.path(), engine, ran, executions.clone());
    run(&cold, directory.path(), &thread, None).await;
    assert_eq!(executions.load(Ordering::SeqCst), 1);
    server.await.unwrap();
}

#[tokio::test]
async fn ordinary_sse_failed_or_disconnected_compaction_never_installs_a_boundary() {
    for failed in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            assert!(
                request(&mut socket)
                    .await
                    .get("context_management")
                    .is_some()
            );
            let mut sse = event(json!({"type":"response.output_item.done","item":opaque()}));
            if failed {
                sse.push_str(&event(json!({"type":"response.failed","response":{"error":{"message":"fixture terminal"}}})));
            }
            reply(&mut socket, &sse).await;
        });
        let engine = Arc::new(OpenAiResponsesEngine::new_with_config(
            Some("fixture".into()),
            "openai",
            base,
            vec![],
        ));
        let live = runtime(
            directory.path(),
            engine,
            Arc::new(Notify::new()),
            Arc::new(AtomicUsize::new(0)),
        );
        live.config
            .write()
            .await
            .reliability
            .provider_retry_max_attempts = 1;
        let thread = live.create_thread(None).await.unwrap().thread_id;
        seed(&live, &thread).await;
        run(&live, directory.path(), &thread, None).await;
        let snapshot = live.load_thread(&thread).await.unwrap().unwrap();
        let items: Vec<_> = snapshot
            .turns
            .into_iter()
            .flat_map(|turn| turn.items)
            .collect();
        assert_original(&items);
        assert!(!items.iter().any(crate::compaction::is_compaction_boundary));
        server.await.unwrap();
    }
}

#[tokio::test]
async fn invalid_or_nested_commit_envelope_is_quarantined_before_real_wire_replay() {
    for envelope in [
        json!({"version":2,"retained_transcript":[]}),
        json!({"version":1,"retained_transcript":[TranscriptItem::ProviderMetadata(json!({"output":[opaque()]}))]}),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let body = request(&mut socket).await;
            assert!(
                body.to_string().contains("original goal evidence"),
                "{body}"
            );
            assert!(
                body.to_string().contains("original tool evidence"),
                "{body}"
            );
            assert!(
                !body["input"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|item| item["type"] == "compaction")
            );
            reply(&mut socket, &final_event()).await;
        });
        let engine = Arc::new(OpenAiResponsesEngine::new_with_config(
            Some("fixture".into()),
            "openai",
            base,
            vec![],
        ));
        let live = runtime(
            directory.path(),
            engine,
            Arc::new(Notify::new()),
            Arc::new(AtomicUsize::new(0)),
        );
        let thread = live.create_thread(None).await.unwrap().thread_id;
        seed(&live, &thread).await;
        live.persist_turn_item(
            &thread,
            &"seed".into(),
            &TranscriptItem::ProviderMetadata(json!({"output":[opaque()],COMMIT_FIELD:envelope})),
        )
        .await
        .unwrap();
        run(&live, directory.path(), &thread, None).await;
        server.await.unwrap();
    }
}
