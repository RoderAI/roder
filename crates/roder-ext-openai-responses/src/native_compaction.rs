use super::*;
use futures::StreamExt;

pub(super) async fn compact(
    engine: &OpenAiResponsesEngine,
    ctx: InferenceTurnContext<'_>,
    mut request: AgentInferenceRequest,
) -> anyhow::Result<Option<InferenceEventStream>> {
    // The endpoint decides model support, including newly released models and
    // custom aliases absent from Roder's catalog. Never substitute a summary.
    if !engine.requires_native_compaction() {
        return Ok(None);
    }
    let key = engine.api_key.as_deref().ok_or_else(|| {
        ProviderFailure::new(
            ProviderFailureKind::Authentication,
            "Responses compaction requires authentication",
        )
    })?;
    // Compaction is its own sampling step, not the parent turn's structured
    // answer or forced tool selection. Match Codex's schema-free prompt.
    request.output = Default::default();
    request.tool_choice = roder_api::tools::ToolChoice::Auto;
    let (mut body, names) = OpenAiResponsesEngine::map_request_with_options(
        &request,
        RequestMappingOptions {
            profile: engine.profile,
            thread_id: Some(ctx.thread_id),
        },
    );
    body.as_object_mut().unwrap().remove("context_management");
    // Codex remote V2 keeps current tools and instructions for both API keys
    // and subscriptions, requesting an opaque boundary on the Responses stream.
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
    Ok(Some(compaction_stream(stream, retained)))
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
    #[tokio::test]
    async fn custom_responses_provider_keeps_local_compaction_fallback() {
        let engine = OpenAiResponsesEngine::new_custom_provider(
            None,
            "custom",
            "Custom",
            "https://custom.example/v1",
        );
        assert!(!engine.requires_native_compaction());
        let stream = engine
            .compact_turn(
                InferenceTurnContext {
                    thread_id: "thread",
                    turn_id: "turn",
                    tool_executor: None,
                },
                super::super::tests::request(),
            )
            .await
            .unwrap();
        assert!(stream.is_none());
    }

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
    async fn api_key_and_codex_compaction_use_streamed_trigger_and_replay_opaque_state() {
        for provider in [PROVIDER_OPENAI, PROVIDER_CODEX] {
            for model in ["gpt-5.5", "uncatalogued-openai-model"] {
                check_streamed_compaction(provider, model).await;
            }
        }
    }

    async fn check_streamed_compaction(provider: &str, model: &str) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let boundary = json!({"id":"compaction1","type":"compaction","encrypted_content":"opaque"});
        let server_boundary = boundary.clone();
        let expected_model = model.to_string();
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
            assert!(header.starts_with("POST /responses HTTP/1.1"), "{header}");
            assert!(
                header
                    .to_lowercase()
                    .contains("authorization: bearer fixture-key")
            );
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
            assert_eq!(body["model"], expected_model);
            assert_eq!(body["instructions"], "be helpful");
            assert_eq!(body["stream"], true);
            assert_eq!(body["store"], false);
            assert_eq!(body["tools"][0]["name"], "echo");
            assert_eq!(body["tool_choice"], "auto");
            assert!(body.get("context_management").is_none());
            assert!(body.get("max_output_tokens").is_none());
            assert!(body["text"].get("format").is_none());
            let input = body["input"].as_array().unwrap();
            assert_eq!(input.len(), 3);
            assert_eq!(input[0]["content"][0]["text"], "Hello");
            assert_eq!(input[1]["content"][0]["text"], "Hi");
            assert_eq!(input.last().unwrap(), &json!({"type":"compaction_trigger"}));
            let done = json!({"type":"response.output_item.done","item":server_boundary});
            let completed = json!({"type":"response.completed","response":{"id":"compact-response","output":[server_boundary],"usage":{"input_tokens":100,"output_tokens":20,"total_tokens":120}}});
            let sse = format!("data: {done}\n\ndata: {completed}\n\n");
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{sse}",sse.len()).as_bytes()).await.unwrap();
        });
        let engine = OpenAiResponsesEngine::new_with_config(
            Some("fixture-key".into()),
            provider,
            base,
            vec![],
        );
        assert!(engine.requires_native_compaction());
        let mut request = crate::provider::tests::request();
        request.model.provider = provider.into();
        request.model.model = model.into();
        // Parent output/tool constraints must not constrain the compaction step.
        request.tool_choice = roder_api::tools::ToolChoice::Specific("echo".into());
        let mut stream = engine
            .compact_turn(
                InferenceTurnContext {
                    thread_id: "real-thread",
                    turn_id: "real-turn",
                    tool_executor: None,
                },
                request.clone(),
            )
            .await
            .unwrap()
            .unwrap();
        let mut completed = 0;
        let mut usage = 0;
        let mut metadata = None;
        while let Some(event) = stream.next().await {
            match event.unwrap() {
                InferenceEvent::ProviderMetadata(value)
                    if value.get("compacted_input").is_some() =>
                {
                    let window = value["compacted_input"].as_array().unwrap();
                    assert_eq!(window.len(), 2);
                    assert_eq!(window[0]["content"][0]["text"], "Hello");
                    assert_eq!(window[1], boundary);
                    metadata = Some(value);
                }
                InferenceEvent::Completed(_) => completed += 1,
                InferenceEvent::Usage(tokens) => usage += tokens.total_tokens,
                _ => {}
            }
        }
        assert_eq!(completed, 1);
        assert_eq!(usage, 120);
        request
            .transcript
            .push(roder_api::transcript::TranscriptItem::ProviderMetadata(
                metadata.unwrap(),
            ));
        request
            .transcript
            .push(roder_api::transcript::TranscriptItem::UserMessage(
                roder_api::transcript::UserMessage::text("continue"),
            ));
        let next = OpenAiResponsesEngine::map_request(&request);
        assert_eq!(next["input"].as_array().unwrap().len(), 3);
        assert_eq!(next["input"][0]["content"][0]["text"], "Hello");
        assert_eq!(next["input"][1], boundary);
        assert_eq!(next["input"][2]["content"][0]["text"], "continue");
        server.await.unwrap();
    }
}
