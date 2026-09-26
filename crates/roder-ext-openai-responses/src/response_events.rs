use super::*;

#[derive(Default)]
pub(super) struct ResponsesStreamState {
    pub(super) terminal: bool,
    pub(super) response_id: Option<String>,
    pub(super) successful: bool,
    pub(super) response_output: Vec<Value>,
    pub(super) streamed_final_text: bool,
    pub(super) current_message_phase: String,
    pub(super) message_phases: HashMap<String, String>,
    pub(super) streamed_message_ids: HashSet<String>,
    pub(super) tool_arguments: HashMap<String, String>,
    pub(super) tool_names: HashMap<String, String>,
    pub(super) tool_call_ids: HashMap<String, String>,
    pub(super) tool_name_map: HashMap<String, String>,
    pub(super) emitted_tool_calls: HashMap<String, ToolCallCompleted>,
    pub(super) protocol_failure: Option<String>,
    pub(super) emitted_hosted_tool_start_ids: HashSet<String>,
    pub(super) emitted_hosted_tool_complete_ids: HashSet<String>,
    pub(super) reasoning_delta_keys: HashSet<String>,
    pub(super) completed_output_ids: HashSet<String>,
    /// Completed `tool_search_call` items without provider-side results:
    /// the client must execute the search and continue the turn.
    pub(super) pending_client_tool_searches: Vec<Value>,
}

pub(super) fn is_client_executed_tool_search(item: &Value) -> bool {
    item.get("type").and_then(Value::as_str) == Some("tool_search_call")
        && item.get("execution").and_then(Value::as_str) == Some("client")
        && item.get("status").and_then(Value::as_str) != Some("failed")
}

#[derive(Debug, PartialEq)]
pub(super) struct SseEvent {
    pub(super) event: Option<String>,
    pub(super) data: Value,
}

// Locate a complete SSE frame without decoding incomplete UTF-8 scalars.
#[cfg(test)]
pub(super) fn take_sse_frame(buffer: &[u8]) -> Option<(Vec<u8>, usize)> {
    let mut line_start = 0;
    let mut cursor = 0;
    take_sse_frame_at(buffer, &mut cursor, &mut line_start)
}

pub(super) fn take_sse_frame_at(
    buffer: &[u8],
    cursor: &mut usize,
    line_start: &mut usize,
) -> Option<(Vec<u8>, usize)> {
    while *cursor < buffer.len() {
        let width = match buffer[*cursor] {
            b'\n' => 1,
            b'\r' => {
                if *cursor + 1 == buffer.len() {
                    return None;
                }
                if buffer[*cursor + 1] == b'\n' { 2 } else { 1 }
            }
            _ => {
                *cursor += 1;
                continue;
            }
        };
        if *cursor == *line_start {
            // Validation happens in parse_sse_bytes; no byte substitution is allowed.
            let frame = Some((buffer[..*line_start].to_vec(), *cursor + width));
            *cursor = 0;
            *line_start = 0;
            return frame;
        }
        *cursor += width;
        *line_start = *cursor;
    }
    None
}

pub(super) fn parse_sse_frame(frame: &str) -> anyhow::Result<Option<SseEvent>> {
    let mut event = None;
    let mut data = Vec::new();

    let normalized = frame.replace("\r\n", "\n").replace('\r', "\n");
    for raw_line in normalized.lines() {
        let line = raw_line.trim_end_matches('\r');
        if let Some(value) = line.strip_prefix("event:") {
            event = Some(value.trim_start().to_string());
        } else if let Some(value) = line.strip_prefix("data:") {
            data.push(value.trim_start());
        }
    }

    if data.is_empty() {
        return Ok(None);
    }

    let data = data.join("\n");
    if data.trim() == "[DONE]" {
        return Ok(None);
    }

    Ok(Some(SseEvent {
        event,
        data: serde_json::from_str(&data).map_err(|err| {
            anyhow::anyhow!(
                "failed to parse Responses SSE data as JSON: {err}; data: {}",
                error_body_excerpt(&data)
            )
        })?,
    }))
}

pub(super) fn events_from_sse_event(
    event: &SseEvent,
    state: &mut ResponsesStreamState,
) -> Vec<InferenceEvent> {
    let kind = event
        .data
        .get("type")
        .and_then(|value| value.as_str())
        .or(event.event.as_deref())
        .unwrap_or_default();

    match kind {
        "response.created" => {
            state.response_id = event
                .data
                .pointer("/response/id")
                .and_then(Value::as_str)
                .map(String::from);
            Vec::new()
        }
        "response.output_text.delta" => {
            let phase = output_text_phase(&event.data, state);
            if let Some(item_id) = event.data.get("item_id").and_then(Value::as_str) {
                state.streamed_message_ids.insert(item_id.to_string());
            }
            event
                .data
                .get("delta")
                .and_then(|value| value.as_str())
                .map(|text| {
                    if is_final_answer_phase(&phase) {
                        state.streamed_final_text = true;
                    }
                    InferenceEvent::MessageDelta(MessageDelta {
                        text: text.to_string(),
                        phase: (!phase.is_empty()).then_some(phase),
                    })
                })
                .into_iter()
                .collect()
        }
        "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
            if let Some(key) = reasoning_content_key(kind, &event.data) {
                state.reasoning_delta_keys.insert(key);
            }
            event
                .data
                .get("delta")
                .and_then(|value| value.as_str())
                .map(|text| {
                    InferenceEvent::ReasoningDelta(ReasoningDelta {
                        text: text.to_string(),
                    })
                })
                .into_iter()
                .collect()
        }
        "response.reasoning_summary_text.done" | "response.reasoning_text.done" => {
            let key = reasoning_content_key(kind, &event.data);
            if key
                .as_ref()
                .is_some_and(|key| state.reasoning_delta_keys.contains(key))
            {
                return Vec::new();
            }
            event
                .data
                .get("text")
                .and_then(|value| value.as_str())
                .map(|text| {
                    InferenceEvent::ReasoningDelta(ReasoningDelta {
                        text: text.to_string(),
                    })
                })
                .into_iter()
                .collect()
        }
        "response.output_item.added" => {
            if let Some(item) = event.data.get("item") {
                record_output_item(item, state);
                if is_compaction_item(item) {
                    return vec![compaction_event(item, "started")];
                }
                if let Some(call) = hosted_tool_call_started_from_item(item) {
                    return emit_hosted_tool_start_once(call, state)
                        .into_iter()
                        .collect();
                }
                if let Some(call) = started_function_call(item, state) {
                    return vec![InferenceEvent::ToolCallStarted(call)];
                }
                if let Some(call) = started_custom_tool_call(item, state) {
                    return vec![InferenceEvent::ToolCallStarted(call)];
                }
            }
            Vec::new()
        }
        "response.web_search_call.searching" => event
            .data
            .get("item_id")
            .and_then(Value::as_str)
            .map(|id| HostedToolCallStarted {
                id: id.to_string(),
                name: "web_search".to_string(),
            })
            .and_then(|call| emit_hosted_tool_start_once(call, state))
            .into_iter()
            .collect(),
        "response.function_call_arguments.delta" | "response.custom_tool_call_input.delta" => {
            if let Some(item_id) = event.data.get("item_id").and_then(Value::as_str)
                && let Some(delta) = event.data.get("delta").and_then(Value::as_str)
            {
                state
                    .tool_arguments
                    .entry(item_id.to_string())
                    .or_default()
                    .push_str(delta);
                let id = state
                    .tool_call_ids
                    .get(item_id)
                    .cloned()
                    .unwrap_or_else(|| item_id.to_string());
                return vec![InferenceEvent::ToolCallDelta(ToolCallDelta {
                    id,
                    arguments_delta: delta.to_string(),
                })];
            }
            Vec::new()
        }
        "response.custom_tool_call_input.done" => {
            if let (Some(id), Some(input)) = (
                event.data.get("item_id").and_then(Value::as_str),
                event.data.get("input").and_then(Value::as_str),
            ) {
                state
                    .tool_arguments
                    .insert(id.to_string(), input.to_string());
            }
            // Tool execution begins only when the output item is complete.
            Vec::new()
        }
        "response.function_call_arguments.done" => {
            if let (Some(id), Some(arguments)) = (
                event.data.get("item_id").and_then(Value::as_str),
                event.data.get("arguments").and_then(Value::as_str),
            ) {
                state
                    .tool_arguments
                    .insert(id.to_string(), arguments.to_string());
            }
            Vec::new()
        }
        "response.output_item.done" => {
            let mut events = Vec::new();
            if let Some(item) = event.data.get("item") {
                let mut item = item.clone();
                let argument_field = if item["type"] == "custom_tool_call" {
                    "input"
                } else {
                    "arguments"
                };
                if matches!(
                    item["type"].as_str(),
                    Some("custom_tool_call" | "function_call")
                ) && item.get(argument_field).is_none()
                    && let Some(input) = item
                        .get("id")
                        .and_then(Value::as_str)
                        .and_then(|id| state.tool_arguments.get(id))
                {
                    item[argument_field] = json!(input);
                }
                let item = &item;
                record_output_item(item, state);
                if is_compaction_item(item) {
                    events.push(compaction_event(item, "completed"));
                }
                if let Some(message) = message_delta_from_done_item(item, state) {
                    events.push(message);
                }
                if is_client_executed_tool_search(item) {
                    // Completion is emitted after the local search runs.
                    state.pending_client_tool_searches.push(item.clone());
                    if let Some(started) = hosted_tool_call_started_from_item(item)
                        && let Some(event) = emit_hosted_tool_start_once(started, state)
                    {
                        events.push(event);
                    }
                } else if let Some(call) = hosted_tool_call_completed_from_item(item) {
                    events.extend(emit_hosted_tool_completed_events(call, state));
                }
                events.extend(
                    extract_tool_calls_from_item(item, &state.tool_name_map)
                        .into_iter()
                        .filter_map(|call| emit_tool_call_once(call, state)),
                );
                if let Some(completed) = completed_output_item(item, state) {
                    events.push(completed);
                }
            }
            events
        }
        "response.completed" => {
            state.terminal = true;
            state.successful = true;
            let response = event.data.get("response").unwrap_or(&event.data);
            state.response_output = response
                .get("output")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let mut events = Vec::new();
            events.extend(message_deltas_from_response(response, state));
            // Recover unstreamed client-executed searches before treating
            // any tool_search_call as a hosted completion.
            if let Some(output) = response.get("output").and_then(Value::as_array) {
                for item in output {
                    if let Some(completed) = completed_output_item(item, state) {
                        events.push(completed);
                    }
                    if is_client_executed_tool_search(item)
                        && !state.pending_client_tool_searches.iter().any(|pending| {
                            pending.get("call_id").or_else(|| pending.get("id"))
                                == item.get("call_id").or_else(|| item.get("id"))
                        })
                    {
                        state.pending_client_tool_searches.push(item.clone());
                        if let Some(started) = hosted_tool_call_started_from_item(item)
                            && let Some(event) = emit_hosted_tool_start_once(started, state)
                        {
                            events.push(event);
                        }
                    }
                }
            }
            for call in extract_hosted_tool_calls(response) {
                events.extend(emit_hosted_tool_completed_events(call, state));
            }
            for call in extract_tool_calls(response, &state.tool_name_map) {
                if let Some(call) = emit_tool_call_once(call, state) {
                    events.push(call);
                }
            }
            if let Some(usage) = extract_usage(response) {
                events.push(InferenceEvent::Usage(usage));
            }
            events.push(InferenceEvent::ProviderMetadata(response.clone()));
            events.push(InferenceEvent::Completed(CompletionMetadata {
                stop_reason: response
                    .get("status")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                provider_response_id: response
                    .get("id")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
            }));
            events
        }
        "response.failed" | "response.incomplete" | "error" => {
            state.terminal = true;
            vec![InferenceEvent::Failed(InferenceFailure {
                message: stream_error_message(&event.data, kind),
            })]
        }
        _ => Vec::new(),
    }
}

fn completed_output_item(item: &Value, state: &mut ResponsesStreamState) -> Option<InferenceEvent> {
    // Providers occasionally omit item ids; use the complete item as the key.
    let key = item
        .get("id")
        .and_then(Value::as_str)
        .map(String::from)
        .unwrap_or_else(|| item.to_string());
    if !state.completed_output_ids.insert(key) || is_compaction_item(item) {
        return None;
    }
    Some(InferenceEvent::OutputItemCompleted(item.clone()))
}

pub(super) fn reasoning_content_key(kind: &str, data: &Value) -> Option<String> {
    let item_id = data.get("item_id").and_then(Value::as_str)?;
    let content_index = data
        .get("content_index")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let kind = kind
        .strip_suffix(".delta")
        .or_else(|| kind.strip_suffix(".done"))
        .unwrap_or(kind);
    Some(format!("{kind}:{item_id}:{content_index}"))
}

pub(super) fn output_text_phase(data: &Value, state: &ResponsesStreamState) -> String {
    data.get("item_id")
        .and_then(Value::as_str)
        .and_then(|item_id| state.message_phases.get(item_id))
        .cloned()
        .unwrap_or_else(|| state.current_message_phase.clone())
}

pub(super) fn is_final_answer_phase(phase: &str) -> bool {
    phase.is_empty() || phase == FINAL_ANSWER_PHASE
}
