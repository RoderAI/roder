use super::*;

pub(super) fn response_input_items(
    request: &AgentInferenceRequest,
    tool_name_map: &ResponsesToolNameMap,
    profile: ResponsesProviderProfile,
    supports_images: bool,
) -> Vec<Value> {
    let mut items = Vec::new();
    let mut explicit_boundary_end = 0;
    let mut provider_output_call_ids = HashSet::new();
    let completed_tool_call_ids = completed_tool_call_ids(&request.transcript);
    let known_tool_call_ids = known_tool_call_ids(&request.transcript);
    let custom_tool_call_ids = custom_tool_call_ids(&request.transcript);
    let mut replayed_item_ids = HashSet::new();
    let search_ids = paired_search_ids(&request.transcript);
    let mut raw_reasoning = raw_reasoning_summaries(&request.transcript);
    let mut raw_messages = if profile == ResponsesProviderProfile::OpenAi {
        raw_message_summaries(&request.transcript)
    } else {
        HashMap::new()
    };
    let raw_calls: HashSet<_> = raw_output(&request.transcript)
        .filter(|item| {
            matches!(
                item["type"].as_str(),
                Some("function_call" | "custom_tool_call")
            )
        })
        .filter_map(|item| item.get("call_id").and_then(Value::as_str))
        .collect();

    for conversation_item in &request.transcript {
        let mapped = match conversation_item {
            roder_api::transcript::TranscriptItem::UserMessage(message) => Some(json!({
                "type": "message",
                "role": "user",
                "content": user_message_content(message, supports_images)
            })),
            roder_api::transcript::TranscriptItem::AssistantMessage(message)
                if !consume_raw_message(&mut raw_messages, message) =>
            {
                Some(json!({
                    "type": "message",
                    "role": "assistant",
                    "phase": message.phase.as_deref().unwrap_or(FINAL_ANSWER_PHASE),
                    "content": [{ "type": "output_text", "text": message.text }]
                }))
            }
            roder_api::transcript::TranscriptItem::ReasoningSummary(summary)
                if !consume_raw_reasoning(&mut raw_reasoning, &summary.text) =>
            {
                Some(json!({
                    "type": "reasoning",
                    "summary": [{ "type": "summary_text", "text": summary.text }]
                }))
            }
            roder_api::transcript::TranscriptItem::ToolCall(call) => {
                if provider_output_call_ids.contains(&call.id)
                    || raw_calls.contains(call.id.as_str())
                    || !completed_tool_call_ids.contains(&call.id)
                {
                    None
                } else {
                    let item_id = fallback_function_call_item_id(&call.id);
                    let name = tool_name_map.replay_api_name(&call.name);
                    Some(json!({
                        "type": "function_call",
                        "id": item_id,
                        "call_id": call.id,
                        "name": name,
                        "arguments": call.arguments,
                        "status": "completed"
                    }))
                }
            }
            roder_api::transcript::TranscriptItem::ToolResult(result) => {
                if known_tool_call_ids.contains(&result.id) {
                    // Results for freeform/custom calls must be replayed as
                    // `custom_tool_call_output`, not `function_call_output`.
                    let is_custom = custom_tool_call_ids.contains(&result.id);
                    let output_type = if is_custom {
                        "custom_tool_call_output"
                    } else {
                        "function_call_output"
                    };
                    // A `view_image` result carries an image content block:
                    // forward it as an `input_image` so the model sees the
                    // pixels. Custom-tool outputs stay plain strings.
                    let output = tool_output_image_block(result)
                        .filter(|_| supports_images && !is_custom)
                        .map(|image| json!([{ "type": "input_image", "image_url": image }]))
                        .unwrap_or_else(|| Value::String(result.result.clone()));
                    Some(json!({
                        "type": output_type,
                        "call_id": result.id,
                        "output": output
                    }))
                } else {
                    None
                }
            }
            roder_api::transcript::TranscriptItem::ContextCompaction(compaction) => Some(json!({
                "type": "message",
                "role": "user",
                "content": [{ "type": "input_text", "text": format!("Context summary:\n{}", compaction.summary) }]
            })),
            roder_api::transcript::TranscriptItem::ProviderMetadata(metadata) => {
                if let Some(window) = metadata.get("compacted_input").and_then(Value::as_array) {
                    items.clear();
                    items.extend(window.iter().cloned());
                    explicit_boundary_end = items.len();
                    provider_output_call_ids.clear();
                    replayed_item_ids.clear();
                    continue;
                }
                append_provider_output_items(
                    metadata,
                    &mut items,
                    &mut provider_output_call_ids,
                    &mut replayed_item_ids,
                    ReplayFilter {
                        completed_tool_call_ids: &completed_tool_call_ids,
                        tool_name_map,
                        profile,
                        completed_search_ids: &search_ids,
                    },
                );
                None
            }
            _ => None,
        };
        if let Some(item) = mapped {
            items.push(item);
        }
    }

    // OpenAI server-side compaction: after the latest compaction item, drop the
    // pre-compact window so the next request does not re-send (and re-compact)
    // the full history. The opaque compaction item carries prior state.
    // https://developers.openai.com/api/docs/guides/compaction
    if let Some(idx) = items.iter().rposition(is_compaction_item)
        && idx >= explicit_boundary_end
    {
        items = items[idx..].to_vec();
    }

    items
}

/// Extract the image `data:` URL a `view_image` tool result stashed in its
/// display payload, so it can be replayed to the model as `input_image`.
pub(super) fn tool_output_image_block(
    result: &roder_api::transcript::ToolResultRecord,
) -> Option<String> {
    result
        .display_payload
        .as_ref()?
        .get(roder_api::transcript::VIEW_IMAGE_DISPLAY_KEY)?
        .get("image_url")?
        .as_str()
        .map(str::to_string)
}

/**
 * Call ids that were emitted on the freeform/custom tool channel, recovered
 * from the raw provider output. Their results replay as `custom_tool_call_output`.
 */
pub(super) fn custom_tool_call_ids(
    transcript: &[roder_api::transcript::TranscriptItem],
) -> HashSet<String> {
    let mut ids = HashSet::new();
    for item in transcript {
        if let roder_api::transcript::TranscriptItem::ProviderMetadata(metadata) = item
            && let Some(output) = metadata.get("output").and_then(Value::as_array)
        {
            for out in output {
                if out.get("type").and_then(Value::as_str) == Some("custom_tool_call")
                    && let Some(call_id) = out.get("call_id").and_then(Value::as_str)
                {
                    ids.insert(call_id.to_string());
                }
            }
        }
    }
    ids
}

pub(super) fn known_tool_call_ids(
    transcript: &[roder_api::transcript::TranscriptItem],
) -> HashSet<String> {
    let mut ids = HashSet::new();
    for item in transcript {
        match item {
            roder_api::transcript::TranscriptItem::ToolCall(call) => {
                ids.insert(call.id.clone());
            }
            roder_api::transcript::TranscriptItem::ProviderMetadata(metadata) => {
                if let Some(output) = metadata.get("output").and_then(Value::as_array) {
                    ids.extend(output.iter().filter_map(|item| {
                        (item.get("type").and_then(Value::as_str) == Some("function_call"))
                            .then(|| {
                                item.get("call_id")
                                    .and_then(Value::as_str)
                                    .map(str::to_string)
                            })
                            .flatten()
                    }));
                }
            }
            _ => {}
        }
    }
    ids
}

pub(super) fn completed_tool_call_ids(
    transcript: &[roder_api::transcript::TranscriptItem],
) -> HashSet<String> {
    transcript
        .iter()
        .filter_map(|item| match item {
            roder_api::transcript::TranscriptItem::ToolResult(result) => Some(result.id.clone()),
            _ => None,
        })
        .collect()
}

pub(super) fn fallback_function_call_item_id(call_id: &str) -> String {
    if call_id.starts_with("fc_") {
        call_id.to_string()
    } else if let Some(suffix) = call_id.strip_prefix("call_") {
        format!("fc_{suffix}")
    } else {
        format!("fc_{call_id}")
    }
}

struct ReplayFilter<'a> {
    completed_tool_call_ids: &'a HashSet<String>,
    tool_name_map: &'a ResponsesToolNameMap,
    profile: ResponsesProviderProfile,
    completed_search_ids: &'a HashSet<String>,
}
fn append_provider_output_items(
    metadata: &Value,
    items: &mut Vec<Value>,
    provider_output_call_ids: &mut HashSet<String>,
    replayed_item_ids: &mut HashSet<String>,
    filter: ReplayFilter<'_>,
) {
    let ReplayFilter {
        completed_tool_call_ids,
        tool_name_map,
        profile,
        completed_search_ids,
    } = filter;
    let Some(output) = metadata.get("output").and_then(Value::as_array) else {
        return;
    };
    for item in output {
        let key = item
            .get("id")
            .and_then(Value::as_str)
            .map(String::from)
            .unwrap_or_else(|| item.to_string());
        if replayed_item_ids.contains(&key) {
            continue;
        }
        let before = items.len();
        match item.get("type").and_then(Value::as_str) {
            Some("function_call") => {
                let call_id = item.get("call_id").and_then(Value::as_str);
                let Some(call_id) = call_id else {
                    continue;
                };
                if !completed_tool_call_ids.contains(call_id) {
                    continue;
                }
                if !provider_output_call_ids.insert(call_id.to_string()) {
                    continue;
                }
                let mut item = item.clone();
                if let Some(name) = item.get("name").and_then(Value::as_str) {
                    item["name"] = json!(tool_name_map.replay_api_name(name));
                }
                items.push(item);
            }
            Some("custom_tool_call") => {
                let Some(call_id) = item.get("call_id").and_then(Value::as_str) else {
                    continue;
                };
                if !completed_tool_call_ids.contains(call_id) {
                    continue;
                }
                if !provider_output_call_ids.insert(call_id.to_string()) {
                    continue;
                }
                let mut item = item.clone();
                if let Some(name) = item.get("name").and_then(Value::as_str) {
                    item["name"] = json!(tool_name_map.replay_api_name(name));
                }
                items.push(item);
            }
            Some("tool_search_call" | "tool_search_output")
                if item
                    .get("call_id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| completed_search_ids.contains(id)) =>
            {
                items.push(item.clone());
            }
            Some("message")
                if profile == ResponsesProviderProfile::OpenAi && item["role"] == "assistant" =>
            {
                items.push(item.clone())
            }
            Some("reasoning") => items.push(replay_provider_output_item(item, profile)),
            Some(kind) if is_compaction_type(kind) => {
                items.push(replay_provider_output_item(item, profile))
            }
            _ => {}
        }
        if items.len() != before {
            replayed_item_ids.insert(key);
        }
    }
}

fn raw_output(
    transcript: &[roder_api::transcript::TranscriptItem],
) -> impl Iterator<Item = &Value> {
    transcript
        .iter()
        .filter_map(|item| match item {
            roder_api::transcript::TranscriptItem::ProviderMetadata(metadata) => {
                metadata.get("output").and_then(Value::as_array)
            }
            _ => None,
        })
        .flatten()
}

fn paired_search_ids(transcript: &[roder_api::transcript::TranscriptItem]) -> HashSet<String> {
    let ids = |kind| {
        raw_output(transcript)
            .filter(move |item| item["type"] == kind)
            .filter_map(|item| item.get("call_id").and_then(Value::as_str))
            .map(String::from)
            .collect::<HashSet<_>>()
    };
    let calls = ids("tool_search_call");
    let outputs = ids("tool_search_output");
    calls.intersection(&outputs).cloned().collect()
}

fn raw_reasoning_summaries(
    transcript: &[roder_api::transcript::TranscriptItem],
) -> HashMap<String, usize> {
    let mut seen = HashSet::new();
    let mut summaries = HashMap::new();
    for item in raw_output(transcript).filter(|item| item["type"] == "reasoning") {
        let key = item
            .get("id")
            .and_then(Value::as_str)
            .map(String::from)
            .unwrap_or_else(|| item.to_string());
        if !seen.insert(key) {
            continue;
        }
        let text: String = item
            .get("summary")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect();
        if !text.is_empty() {
            *summaries.entry(text).or_default() += 1;
        }
    }
    summaries
}

fn consume_raw_reasoning(summaries: &mut HashMap<String, usize>, text: &str) -> bool {
    if let Some(count) = summaries.get_mut(text)
        && *count > 0
    {
        *count -= 1;
        return true;
    }
    false
}

pub(super) fn replay_provider_output_item(
    item: &Value,
    profile: ResponsesProviderProfile,
) -> Value {
    let mut item = item.clone();
    if profile == ResponsesProviderProfile::Fireworks
        && let Some(object) = item.as_object_mut()
    {
        object.remove("encrypted_content");
    }
    item
}

fn raw_message_summaries(
    transcript: &[roder_api::transcript::TranscriptItem],
) -> HashMap<(String, String), usize> {
    let mut seen = HashSet::new();
    let mut messages = HashMap::new();
    for item in raw_output(transcript)
        .filter(|item| item["type"] == "message" && item["role"] == "assistant")
    {
        let key = item
            .get("id")
            .and_then(Value::as_str)
            .map(String::from)
            .unwrap_or_else(|| item.to_string());
        if !seen.insert(key) {
            continue;
        }
        let text: String = item
            .get("content")
            .and_then(Value::as_str)
            .map(String::from)
            .unwrap_or_else(|| {
                item.get("content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|block| block.get("text").and_then(Value::as_str))
                    .collect()
            });
        let phase = item
            .get("phase")
            .and_then(Value::as_str)
            .unwrap_or(FINAL_ANSWER_PHASE)
            .to_string();
        *messages.entry((phase, text)).or_default() += 1;
    }
    messages
}

fn consume_raw_message(
    messages: &mut HashMap<(String, String), usize>,
    message: &roder_api::transcript::AssistantMessage,
) -> bool {
    let key = (
        message
            .phase
            .as_deref()
            .unwrap_or(FINAL_ANSWER_PHASE)
            .to_string(),
        message.text.clone(),
    );
    if let Some(count) = messages.get_mut(&key)
        && *count > 0
    {
        *count -= 1;
        true
    } else {
        false
    }
}
