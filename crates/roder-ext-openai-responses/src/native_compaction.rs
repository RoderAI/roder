use super::*;
use futures::StreamExt;

pub(super) async fn compact(
    engine: &OpenAiResponsesEngine,
    ctx: InferenceTurnContext<'_>,
    request: AgentInferenceRequest,
) -> anyhow::Result<Option<InferenceEventStream>> {
    if engine.profile != ResponsesProviderProfile::OpenAi
        || !lookup_model_for_provider(&request.model.provider, &request.model.model)
            .is_some_and(|model| model.supports_compaction)
    {
        return Ok(None);
    }
    let key = engine.api_key.as_deref().ok_or_else(|| {
        ProviderFailure::new(
            ProviderFailureKind::Authentication,
            "Responses compaction requires authentication",
        )
    })?;
    let (mut body, names) = OpenAiResponsesEngine::map_request_with_options(
        &request,
        RequestMappingOptions {
            profile: engine.profile,
            thread_id: Some(ctx.thread_id),
        },
    );
    body.as_object_mut().unwrap().remove("context_management");
    if engine.provider_id == PROVIDER_CODEX {
        // Current Codex remote V2 keeps model-visible tools/instructions and
        // requests an opaque boundary through the normal Responses stream.
        let retained = retained_client_messages(&body);
        body["input"]
            .as_array_mut()
            .unwrap()
            .push(json!({"type":"compaction_trigger"}));
        let stream = if websocket_requested(&engine.base_url) {
            try_websocket_stream(
                &engine.base_url,
                key,
                &engine.headers,
                ctx.thread_id,
                &body,
                names.api_name_to_tool_name.clone(),
                None,
            )
            .await?
        } else {
            None
        };
        let stream = match stream {
            Some(stream) => stream,
            None => {
                let response = send_responses_request(
                    &engine.base_url,
                    key,
                    &engine.headers,
                    None,
                    &body,
                    request.runtime.reliability.as_ref(),
                )
                .await?;
                stream_responses_sse_with_client_tool_search(
                    response.response,
                    names.api_name_to_tool_name,
                    response.retry_events,
                    response.idle_timeout,
                    None,
                )
            }
        };
        return Ok(Some(compaction_stream(stream, retained)));
    }
    // Standalone API compaction returns the complete next window. Keep its
    // retained items verbatim, including messages before the encrypted item.
    let mut compact_body = json!({"model":body["model"],"input":body["input"]});
    if let Some(instructions) = body.get("instructions") {
        compact_body["instructions"] = instructions.clone();
    }
    let response = send_responses_endpoint(
        &engine.base_url,
        key,
        &engine.headers,
        None,
        &compact_body,
        request.runtime.reliability.as_ref(),
        ResponseEndpoint {
            idle_timeout: responses_stream_idle_timeout(),
            path: "responses/compact",
        },
    )
    .await?;
    let retry_events = response.retry_events;
    let mut chunks = response.response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk.map_err(|error| {
            ProviderFailure::new(ProviderFailureKind::StreamInterrupted, error.to_string())
        })?;
        if bytes.len().saturating_add(chunk.len()) > 16 * 1024 * 1024 {
            return Err(ProviderFailure::new(
                ProviderFailureKind::Protocol,
                "compaction response exceeds 16 MiB",
            )
            .into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|error| {
        ProviderFailure::new(
            ProviderFailureKind::Protocol,
            format!("invalid compaction response: {error}"),
        )
    })?;
    let output = value
        .get("output")
        .and_then(Value::as_array)
        .filter(|output| {
            output
                .iter()
                .filter(|item| is_compaction_item(item))
                .count()
                == 1
        })
        .ok_or_else(|| {
            ProviderFailure::new(
                ProviderFailureKind::Protocol,
                "compaction response must contain exactly one opaque boundary",
            )
        })?
        .clone();
    let metadata = json!({"output":output,"compacted_input":output});
    let mut events: Vec<anyhow::Result<InferenceEvent>> = retry_events
        .into_iter()
        .map(|metadata| Ok(InferenceEvent::ProviderMetadata(metadata)))
        .collect();
    if let Some(usage) = extract_usage(&value) {
        events.push(Ok(InferenceEvent::Usage(usage)));
    }
    events.push(Ok(InferenceEvent::ProviderMetadata(metadata)));
    events.push(Ok(InferenceEvent::Completed(CompletionMetadata {
        stop_reason: Some("completed".into()),
        provider_response_id: value.get("id").and_then(Value::as_str).map(String::from),
    })));
    Ok(Some(Box::pin(futures::stream::iter(events))))
}

fn retained_client_messages(body: &Value) -> Vec<Value> {
    // Codex retains recent client-authored messages alongside the boundary.
    // Budget text conservatively; image byte limits are enforced at dispatch.
    let mut chars = 0;
    let mut retained: Vec<_> = body["input"]
        .as_array()
        .into_iter()
        .flatten()
        .rev()
        .filter(|item| {
            item["type"] == "message"
                && matches!(item["role"].as_str(), Some("user" | "developer" | "system"))
        })
        .filter(|item| {
            let count = item.to_string().len();
            if chars > 0 && chars + count > 40_000 {
                false
            } else {
                chars += count;
                true
            }
        })
        .cloned()
        .collect();
    retained.reverse();
    retained
}

fn compaction_stream(
    mut stream: InferenceEventStream,
    retained: Vec<Value>,
) -> InferenceEventStream {
    Box::pin(async_stream::try_stream! {
        let mut boundary = None;
        while let Some(event) = stream.next().await {
            match event? {
                InferenceEvent::Compaction(progress) => {
                    if progress.status == "completed" && let Some(item) = progress.item.clone() {
                        if boundary.as_ref().is_some_and(|prior:&Value|prior.get("id") != item.get("id") || prior.get("encrypted_content") != item.get("encrypted_content")) {
                            Err(ProviderFailure::new(ProviderFailureKind::Protocol,"compaction response contains multiple opaque boundaries"))?;
                        }
                        if boundary.is_none() {
                            boundary = Some(item.clone());
                            let mut window = retained.clone();
                            window.push(item.clone());
                            // Durable even if the stream breaks before Completed.
                            yield InferenceEvent::ProviderMetadata(json!({"output":[item],"compacted_input":window}));
                        }
                    }
                    yield InferenceEvent::Compaction(progress);
                }
                InferenceEvent::Completed(completion) => {
                    if boundary.is_none() { Err(ProviderFailure::new(ProviderFailureKind::Protocol,"compaction completed without an opaque boundary"))?; }
                    yield InferenceEvent::Completed(completion);
                    return;
                }
                InferenceEvent::ToolCallCompleted(_) => {
                    Err(ProviderFailure::new(ProviderFailureKind::Protocol,"compaction emitted an executable tool call"))?;
                }
                InferenceEvent::Usage(usage) => yield InferenceEvent::Usage(usage),
                InferenceEvent::ProviderMetadata(metadata) if metadata.get("output").is_none() => yield InferenceEvent::ProviderMetadata(metadata),
                InferenceEvent::Failed(failure) => Err(ProviderFailure::new(ProviderFailureKind::Protocol,failure.message))?,
                _ => {}
            }
        }
        Err(ProviderFailure::new(ProviderFailureKind::StreamInterrupted,"compaction stream ended before terminal completion"))?;
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compacted_window_replays_all_retained_items_without_pruning_before_boundary() {
        use roder_api::transcript::{TranscriptItem, UserMessage};
        let mut request = super::super::tests::request();
        let window = json!([
            {"type":"message","role":"user","content":[{"type":"input_text","text":"retain goal"}]},
            {"type":"compaction","id":"boundary","encrypted_content":"opaque"},
            {"type":"message","role":"assistant","content":[{"type":"output_text","text":"retained state"}]}
        ]);
        request.transcript = vec![
            TranscriptItem::UserMessage(UserMessage::text("old history")),
            TranscriptItem::ProviderMetadata(json!({"output":window,"compacted_input":window})),
            TranscriptItem::UserMessage(UserMessage::text("new instruction")),
        ];
        let body = OpenAiResponsesEngine::map_request(&request);
        assert_eq!(body["input"][0], window[0]);
        assert_eq!(body["input"][1], window[1]);
        assert_eq!(body["input"][2], window[2]);
        assert_eq!(body["input"].as_array().unwrap().len(), 4);
    }
    #[tokio::test]
    async fn in_progress_compaction_is_not_committed_and_duplicate_done_is_deduplicated() {
        let boundary = json!({"type":"compaction","id":"boundary","encrypted_content":"opaque"});
        let progress = |status: &str, item: Value| {
            InferenceEvent::Compaction(CompactionProgress {
                status: status.into(),
                item_id: Some("boundary".into()),
                tokens_before: None,
                tokens_after: None,
                duration_ms: None,
                item: Some(item),
            })
        };
        let events = vec![
            progress(
                "in_progress",
                json!({"type":"compaction","id":"boundary","status":"in_progress"}),
            ),
            progress("completed", boundary.clone()),
            progress("completed", boundary),
            InferenceEvent::Completed(CompletionMetadata {
                stop_reason: Some("completed".into()),
                provider_response_id: None,
            }),
        ];
        let stream: InferenceEventStream =
            Box::pin(futures::stream::iter(events.into_iter().map(Ok)));
        let events: Vec<_> = compaction_stream(stream, vec![])
            .map(|event| event.unwrap())
            .collect()
            .await;
        assert!(matches!(events[0], InferenceEvent::Compaction(_)));
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, InferenceEvent::ProviderMetadata(_)))
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, InferenceEvent::Completed(_)))
                .count(),
            1
        );
    }
    #[tokio::test]
    async fn codex_compaction_boundary_is_durable_before_disconnect() {
        let boundary = json!({"type":"compaction","id":"boundary","encrypted_content":"opaque"});
        let progress = CompactionProgress {
            status: "completed".into(),
            item_id: Some("boundary".into()),
            tokens_before: None,
            tokens_after: None,
            duration_ms: None,
            item: Some(boundary.clone()),
        };
        let stream: InferenceEventStream = Box::pin(futures::stream::iter(vec![
            Ok(InferenceEvent::Compaction(progress)),
            Err(ProviderFailure::new(ProviderFailureKind::StreamInterrupted, "closed").into()),
        ]));
        let retained = vec![json!({"type":"message","role":"user","content":"goal"})];
        let mut stream = compaction_stream(stream, retained.clone());
        let InferenceEvent::ProviderMetadata(metadata) = stream.next().await.unwrap().unwrap()
        else {
            panic!("durable window missing")
        };
        assert_eq!(metadata["compacted_input"], json!([retained[0], boundary]));
        stream.next().await.unwrap().unwrap();
        assert_eq!(
            stream
                .next()
                .await
                .unwrap()
                .unwrap_err()
                .downcast_ref::<ProviderFailure>()
                .unwrap()
                .kind,
            ProviderFailureKind::StreamInterrupted
        );
    }
}

#[cfg(test)]
mod http_tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    #[tokio::test]
    async fn standalone_api_compaction_sends_full_window_and_preserves_response_verbatim() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let window = json!([{ "type":"message","role":"user","content":"retain me" },
            {"id":"compaction1","type":"compaction","encrypted_content":"opaque"}]);
        let server_window = window.clone();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut buf = [0; 4096];
            let header_end = loop {
                let n = socket.read(&mut buf).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
                if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                    break end + 4;
                }
            };
            let header = String::from_utf8_lossy(&bytes[..header_end]);
            assert!(header.starts_with("POST /responses/compact"));
            let length: usize = header
                .lines()
                .find_map(|line| {
                    line.to_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|s| s.trim().parse().ok())
                })
                .unwrap();
            while bytes.len() < header_end + length {
                let n = socket.read(&mut buf).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
            }
            let body: Value = serde_json::from_slice(&bytes[header_end..]).unwrap();
            assert_eq!(body["model"], "gpt-5.5");
            assert_eq!(body["instructions"], "be helpful");
            assert!(body["input"].as_array().unwrap().len() >= 2);
            assert!(body.get("stream").is_none());
            assert!(body.get("tools").is_none());
            let body = json!({"id":"compact-response","output":server_window,"usage":{"input_tokens":100,"output_tokens":20,"total_tokens":120}}).to_string();
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
        });
        let engine = OpenAiResponsesEngine::new_with_config(
            Some("fixture-key".into()),
            "openai",
            base,
            vec![],
        );
        let request = crate::provider::tests::request();
        let mut stream = engine
            .compact_turn(
                InferenceTurnContext {
                    thread_id: "real-thread",
                    turn_id: "real-turn",
                    tool_executor: None,
                },
                request,
            )
            .await
            .unwrap()
            .unwrap();
        let mut completed = 0;
        let mut usage = 0;
        while let Some(event) = stream.next().await {
            match event.unwrap() {
                InferenceEvent::ProviderMetadata(metadata)
                    if metadata.get("compacted_input").is_some() =>
                {
                    assert_eq!(metadata["compacted_input"], window)
                }
                InferenceEvent::Completed(_) => completed += 1,
                InferenceEvent::Usage(tokens) => usage += tokens.total_tokens,
                _ => {}
            }
        }
        assert_eq!(completed, 1);
        assert_eq!(usage, 120);
        server.await.unwrap();
    }
}
