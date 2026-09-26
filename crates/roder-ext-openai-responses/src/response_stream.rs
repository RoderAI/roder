use super::*;

/// Bounded number of in-turn client-executed tool-search continuations.
pub(super) const MAX_CLIENT_TOOL_SEARCH_ROUNDS: usize = 3;

pub(super) fn stream_responses_sse_with_client_tool_search(
    response: reqwest::Response,
    tool_name_map: HashMap<String, String>,
    retry_events: Vec<Value>,
    mut idle_timeout: Duration,
    mut continuation: Option<ClientToolSearchContext>,
) -> InferenceEventStream {
    Box::pin(async_stream::try_stream! {
        use futures::StreamExt as _;

        for retry_event in retry_events {
            yield InferenceEvent::ProviderMetadata(retry_event);
        }

        let mut response = response;
        let mut search_history = Vec::new();
        let mut state = ResponsesStreamState {
            tool_name_map: tool_name_map.clone(),
            ..Default::default()
        };
        for round in 0..=MAX_CLIENT_TOOL_SEARCH_ROUNDS {
            let diagnostics = ResponseStreamDiagnostics::from_response(&response, idle_timeout);
            let mut chunks = response.bytes_stream();
            let mut buffer = Vec::new();
            let mut cursor = 0;
            let mut line_start = 0;
            state.terminal = false;
            state.response_id = None;
            state.successful = false;
            state.streamed_final_text = false;
            state.current_message_phase.clear();
            state.response_output.clear();

            while let Some(chunk) = chunks.next().await {
                let chunk = chunk.map_err(|error| {
                    let mut failure = diagnostics.attach(ProviderFailure::new(ProviderFailureKind::StreamInterrupted, diagnostics.read_error(error).to_string()));
                    failure.response_id = state.response_id.clone();
                    failure
                })?;
                buffer.extend_from_slice(&chunk);
                while let Some((frame, consumed)) = take_sse_frame_at(&buffer, &mut cursor, &mut line_start) {
                    validate_sse_frame_size(frame.len())?;
                    buffer.drain(..consumed);
                    let Some(event) = decode_frame(&frame)? else {
                        continue;
                    };
                    for mut inference_event in events_from_sse_event(&event, &mut state) {
                        // A pending client search means this turn continues
                        // with a follow-up request; the intermediate
                        // completion must not terminate the canonical turn.
                        if matches!(inference_event, InferenceEvent::Completed(_))
                            && continuation.is_some()
                            && !state.pending_client_tool_searches.is_empty()
                        {
                            continue;
                        }
                        if let InferenceEvent::ProviderMetadata(metadata) = &mut inference_event
                            && metadata.get("output").is_some() {
                            if !state.pending_client_tool_searches.is_empty() { continue; }
                            let mut output = search_history.clone();
                            output.extend(state.response_output.clone());
                            metadata["output"] = json!(output);
                        }
                        if matches!(inference_event, InferenceEvent::Failed(_)) {
                            if let Some(message) = state.protocol_failure.take() { Err(ProviderFailure::new(ProviderFailureKind::Protocol,message))?; }
                        let mut failure = diagnostics.attach(sse_failure(&event.data,event.event.as_deref().unwrap_or("response.failed")));
                        if failure.response_id.is_none() { failure.response_id = state.response_id.clone(); }
                        Err(failure)?;
                        }
                        yield inference_event;
                    }
                    if state.terminal { break; }
                }
                if state.terminal { break; }
                validate_sse_frame_size(buffer.len())?;
            }

            let trailing_event = if state.terminal || buffer.iter().all(u8::is_ascii_whitespace) {
                None
            } else {
                decode_frame(&buffer)?
            };
            if let Some(event) = trailing_event {
                for mut inference_event in events_from_sse_event(&event, &mut state) {
                    if matches!(inference_event, InferenceEvent::Completed(_))
                        && continuation.is_some()
                        && !state.pending_client_tool_searches.is_empty()
                    {
                        continue;
                    }
                    if let InferenceEvent::ProviderMetadata(metadata) = &mut inference_event
                        && metadata.get("output").is_some() {
                        if !state.pending_client_tool_searches.is_empty() { continue; }
                        let mut output = search_history.clone();
                        output.extend(state.response_output.clone());
                        metadata["output"] = json!(output);
                    }
                    if matches!(inference_event, InferenceEvent::Failed(_)) {
                        if let Some(message) = state.protocol_failure.take() { Err(ProviderFailure::new(ProviderFailureKind::Protocol,message))?; }
                        let mut failure = diagnostics.attach(sse_failure(&event.data,event.event.as_deref().unwrap_or("response.failed")));
                        if failure.response_id.is_none() { failure.response_id = state.response_id.clone(); }
                        Err(failure)?;
                    }
                    yield inference_event;
                }
            }

            if !state.terminal {
                let mut failure = diagnostics.attach(ProviderFailure::new(ProviderFailureKind::StreamInterrupted,"stream closed before response.completed"));
                failure.response_id = state.response_id.clone();
                Err(failure)?;
            }

            if !state.successful { break; }
            let pending = std::mem::take(&mut state.pending_client_tool_searches);
            let Some(ctx) = continuation.as_mut() else {
                break;
            };
            if pending.is_empty() {
                break;
            }

            if round == MAX_CLIENT_TOOL_SEARCH_ROUNDS {
                Err(ProviderFailure::new(ProviderFailureKind::ToolSearchExhausted, "Responses client tool-search continuation limit exhausted"))?;
            }
            let execution = execute_client_searches(ctx, &mut state, pending)?;
            search_history.extend(execution.history);
            for event in execution.events { yield event; }

            let retried = send_responses_request(
                &ctx.base_url,
                &ctx.api_key,
                &ctx.headers,
                ctx.grok_conversation_id.as_deref(),
                &ctx.body,
                ctx.policy.as_ref(),
            )
            .await?;
            for retry_event in retried.retry_events {
                yield InferenceEvent::ProviderMetadata(retry_event);
            }
            idle_timeout = retried.idle_timeout;
            response = retried.response;
        }
    })
}

fn validate_sse_frame_size(bytes: usize) -> anyhow::Result<()> {
    if bytes > 16 * 1024 * 1024 {
        return Err(ProviderFailure::new(
            ProviderFailureKind::Protocol,
            "Responses SSE frame exceeds 16 MiB",
        )
        .into());
    }
    Ok(())
}

fn decode_frame(bytes: &[u8]) -> anyhow::Result<Option<SseEvent>> {
    let text = std::str::from_utf8(bytes).map_err(|error| {
        ProviderFailure::new(
            ProviderFailureKind::Protocol,
            format!("invalid Responses UTF-8: {error}"),
        )
    })?;
    parse_sse_frame(text).map_err(|error| {
        ProviderFailure::new(
            ProviderFailureKind::Protocol,
            format!("invalid Responses SSE data: {error}"),
        )
        .into()
    })
}
