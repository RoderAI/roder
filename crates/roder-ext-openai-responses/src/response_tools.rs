use super::*;

pub(super) fn hosted_tool_call_started_from_item(item: &Value) -> Option<HostedToolCallStarted> {
    let name = hosted_tool_name(item)?;
    let id = item.get("id").and_then(Value::as_str)?;
    Some(HostedToolCallStarted {
        id: id.to_string(),
        name: name.to_string(),
    })
}

pub(super) fn hosted_tool_call_completed_from_item(
    item: &Value,
) -> Option<HostedToolCallCompleted> {
    let name = hosted_tool_name(item)?;
    let id = item.get("id").and_then(Value::as_str)?;
    Some(HostedToolCallCompleted {
        id: id.to_string(),
        name: name.to_string(),
        arguments: hosted_tool_arguments(item),
    })
}

pub(super) fn hosted_tool_name(item: &Value) -> Option<&'static str> {
    match item.get("type").and_then(Value::as_str) {
        Some("web_search_call") => Some("web_search"),
        Some("tool_search_call") => Some("tool_search"),
        _ => None,
    }
}

pub(super) fn hosted_tool_arguments(item: &Value) -> String {
    let mut arguments = serde_json::Map::new();
    if let Some(action) = item.get("action").and_then(Value::as_object) {
        if let Some(action_type) = action.get("type").and_then(Value::as_str) {
            arguments.insert("action".to_string(), Value::String(action_type.to_string()));
        }
        if let Some(query) = action
            .get("query")
            .or_else(|| action.get("pattern"))
            .and_then(Value::as_str)
        {
            arguments.insert("query".to_string(), Value::String(query.to_string()));
        } else if let Some(queries) = action.get("queries").and_then(Value::as_array)
            && let Some(query) = queries.first().and_then(Value::as_str)
        {
            arguments.insert("query".to_string(), Value::String(query.to_string()));
        }
        if let Some(url) = action.get("url").and_then(Value::as_str) {
            arguments.insert("url".to_string(), Value::String(url.to_string()));
        }
    }
    // tool_search_call items carry the search query and the searched tool
    // selection at the item level; preserve them so the canonical hosted
    // tool-call events never lose searched tool ids.
    if !arguments.contains_key("query") {
        if let Some(query) = item.get("query").and_then(Value::as_str) {
            arguments.insert("query".to_string(), Value::String(query.to_string()));
        } else if let Some(queries) = item.get("queries").and_then(Value::as_array)
            && let Some(query) = queries.first().and_then(Value::as_str)
        {
            arguments.insert("query".to_string(), Value::String(query.to_string()));
        }
    }
    if let Some(results) = item.get("results").and_then(Value::as_array) {
        let selected: Vec<Value> = results
            .iter()
            .filter_map(|result| {
                result
                    .get("name")
                    .or_else(|| result.get("tool_name"))
                    .and_then(Value::as_str)
                    .map(|name| Value::String(name.to_string()))
            })
            .collect();
        if !selected.is_empty() {
            arguments.insert("selected_tools".to_string(), Value::Array(selected));
        }
    }
    Value::Object(arguments).to_string()
}

pub(super) fn emit_hosted_tool_start_once(
    call: HostedToolCallStarted,
    state: &mut ResponsesStreamState,
) -> Option<InferenceEvent> {
    state
        .emitted_hosted_tool_start_ids
        .insert(call.id.clone())
        .then_some(InferenceEvent::HostedToolCallStarted(call))
}

pub(super) fn emit_hosted_tool_completed_events(
    call: HostedToolCallCompleted,
    state: &mut ResponsesStreamState,
) -> Vec<InferenceEvent> {
    let mut events = Vec::new();
    if let Some(started) = emit_hosted_tool_start_once(
        HostedToolCallStarted {
            id: call.id.clone(),
            name: call.name.clone(),
        },
        state,
    ) {
        events.push(started);
    }
    if state
        .emitted_hosted_tool_complete_ids
        .insert(call.id.clone())
    {
        events.push(InferenceEvent::HostedToolCallCompleted(call));
    }
    events
}

pub(super) fn record_output_item(item: &Value, state: &mut ResponsesStreamState) {
    match item.get("type").and_then(Value::as_str) {
        Some("message") => {
            let phase = item
                .get("phase")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            state.current_message_phase = phase.clone();
            if let Some(id) = item.get("id").and_then(Value::as_str) {
                state.message_phases.insert(id.to_string(), phase);
            }
        }
        Some("function_call") => {
            let Some(id) = item.get("id").and_then(Value::as_str) else {
                return;
            };
            if let Some(name) = item.get("name").and_then(Value::as_str) {
                state.tool_names.insert(id.to_string(), name.to_string());
            }
            if let Some(call_id) = item
                .get("call_id")
                .or_else(|| item.get("id"))
                .and_then(Value::as_str)
            {
                state
                    .tool_call_ids
                    .insert(id.to_string(), call_id.to_string());
            }
            if let Some(arguments) = item.get("arguments").and_then(Value::as_str) {
                state
                    .tool_arguments
                    .insert(id.to_string(), arguments.to_string());
            }
        }
        Some("custom_tool_call") => {
            let Some(id) = item.get("id").and_then(Value::as_str) else {
                return;
            };
            if let Some(name) = item.get("name").and_then(Value::as_str) {
                state.tool_names.insert(id.to_string(), name.to_string());
            }
            if let Some(call_id) = item
                .get("call_id")
                .or_else(|| item.get("id"))
                .and_then(Value::as_str)
            {
                state
                    .tool_call_ids
                    .insert(id.to_string(), call_id.to_string());
            }
        }
        _ => {}
    }
}

pub(super) fn message_delta_from_done_item(
    item: &Value,
    state: &mut ResponsesStreamState,
) -> Option<InferenceEvent> {
    if item.get("type").and_then(Value::as_str) != Some("message") {
        return None;
    }
    let id = item.get("id").and_then(Value::as_str);
    if id.is_some_and(|id| state.streamed_message_ids.contains(id)) {
        return None;
    }
    let text = output_text_from_message_item(item)?;
    let phase = item
        .get("phase")
        .and_then(Value::as_str)
        .unwrap_or(FINAL_ANSWER_PHASE)
        .to_string();
    if is_final_answer_phase(&phase) {
        if state.streamed_final_text {
            return None;
        }
        state.streamed_final_text = true;
    }
    if let Some(id) = id {
        state.streamed_message_ids.insert(id.to_string());
    }
    Some(InferenceEvent::MessageDelta(MessageDelta {
        text,
        phase: Some(phase),
    }))
}

pub(super) fn started_function_call(
    item: &Value,
    state: &ResponsesStreamState,
) -> Option<ToolCallStarted> {
    if item.get("type").and_then(Value::as_str) != Some("function_call") {
        return None;
    }
    let item_id = item.get("id").and_then(Value::as_str)?;
    let id = state
        .tool_call_ids
        .get(item_id)
        .cloned()
        .unwrap_or_else(|| item_id.to_string());
    let name = item
        .get("name")
        .and_then(Value::as_str)
        .map(|name| map_tool_name(name, &state.tool_name_map).to_string())
        .or_else(|| {
            state
                .tool_names
                .get(item_id)
                .map(|name| map_tool_name(name, &state.tool_name_map).to_string())
        })?;
    Some(ToolCallStarted { id, name })
}

pub(super) fn started_custom_tool_call(
    item: &Value,
    state: &ResponsesStreamState,
) -> Option<ToolCallStarted> {
    if item.get("type").and_then(Value::as_str) != Some("custom_tool_call") {
        return None;
    }
    let item_id = item.get("id").and_then(Value::as_str)?;
    let id = state
        .tool_call_ids
        .get(item_id)
        .cloned()
        .unwrap_or_else(|| item_id.to_string());
    let name = item
        .get("name")
        .and_then(Value::as_str)
        .map(|name| map_tool_name(name, &state.tool_name_map).to_string())
        .or_else(|| {
            state
                .tool_names
                .get(item_id)
                .map(|name| map_tool_name(name, &state.tool_name_map).to_string())
        })?;
    Some(ToolCallStarted { id, name })
}

pub(super) fn emit_tool_call_once(
    call: ToolCallCompleted,
    state: &mut ResponsesStreamState,
) -> Option<InferenceEvent> {
    if let Some(previous) = state.emitted_tool_calls.get(&call.id) {
        let same = previous.arguments == call.arguments
            || serde_json::from_str::<Value>(&previous.arguments)
                .ok()
                .zip(serde_json::from_str::<Value>(&call.arguments).ok())
                .is_some_and(|(left, right)| left == right);
        if previous.name != call.name || !same {
            let message = format!(
                "provider reused tool call id {} with different arguments",
                call.id
            );
            state.protocol_failure = Some(message.clone());
            return Some(InferenceEvent::Failed(InferenceFailure { message }));
        }
        return None;
    }
    state
        .emitted_tool_calls
        .insert(call.id.clone(), call.clone());
    Some(InferenceEvent::ToolCallCompleted(call))
}

/// Normalize raw custom-tool input to the tool's canonical dispatch field.
pub(super) fn custom_tool_call_completed(
    item: &Value,
    tool_name_map: &HashMap<String, String>,
) -> Option<ToolCallCompleted> {
    if item.get("type").and_then(Value::as_str) != Some("custom_tool_call") {
        return None;
    }
    let id = item
        .get("call_id")
        .or_else(|| item.get("id"))
        .and_then(Value::as_str)?;
    let name = item.get("name").and_then(Value::as_str)?;
    let input = item
        .get("input")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let name = map_tool_name(name, tool_name_map);
    let field = roder_api::tools::ToolSpec::freeform_field_for_name(name)?;
    Some(ToolCallCompleted {
        id: id.to_string(),
        name: name.to_string(),
        arguments: json!({ field: input }).to_string(),
    })
}

pub(super) fn extract_tool_calls_from_item(
    item: &Value,
    tool_name_map: &HashMap<String, String>,
) -> Vec<ToolCallCompleted> {
    if let Some(call) = custom_tool_call_completed(item, tool_name_map) {
        return vec![call];
    }
    if item.get("type").and_then(|value| value.as_str()) != Some("function_call") {
        return Vec::new();
    }

    let Some(id) = item
        .get("call_id")
        .or_else(|| item.get("id"))
        .and_then(|value| value.as_str())
    else {
        return Vec::new();
    };
    let Some(name) = item.get("name").and_then(|value| value.as_str()) else {
        return Vec::new();
    };
    vec![ToolCallCompleted {
        id: id.to_string(),
        name: map_tool_name(name, tool_name_map).to_string(),
        arguments: item
            .get("arguments")
            .and_then(|value| value.as_str())
            .unwrap_or("{}")
            .to_string(),
    }]
}
