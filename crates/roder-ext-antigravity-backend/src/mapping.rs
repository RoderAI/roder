use roder_api::backend::{BackendEvent, BackendModel};
use roder_api::inference::TokenUsage;
use serde_json::Value;

pub(crate) fn text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(items) => items.iter().map(text).collect::<Vec<_>>().join("\n"),
        Value::Object(_) => {
            if let Some(content) = value.get("content") {
                text(content)
            } else {
                value["text"].as_str().unwrap_or("").to_owned()
            }
        }
        _ => String::new(),
    }
}

pub(crate) fn choices(option: &Value) -> Vec<&Value> {
    option["options"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|entry| {
            entry["options"]
                .as_array()
                .map(|group| group.iter().collect())
                .unwrap_or_else(|| vec![entry])
        })
        .collect()
}

pub(crate) fn config<'a>(state: &'a Value, category: &str) -> Option<&'a Value> {
    state["configOptions"].as_array()?.iter().find(|option| {
        option["id"].as_str() == Some(category) || option["category"].as_str() == Some(category)
    })
}

pub(crate) fn models(state: &Value) -> Vec<BackendModel> {
    let thinking = config(state, "thought_level");
    let default_reasoning = thinking
        .and_then(|option| option["currentValue"].as_str())
        .unwrap_or("");
    let reasoning_efforts = thinking
        .map(choices)
        .unwrap_or_default()
        .iter()
        .filter_map(|entry| entry["value"].as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    if let Some(option) = config(state, "model") {
        return choices(option)
            .into_iter()
            .filter_map(|entry| {
                let id = entry["value"].as_str()?;
                Some(BackendModel {
                    id: id.into(),
                    name: entry["name"].as_str().unwrap_or(id).into(),
                    is_default: option["currentValue"].as_str() == Some(id),
                    default_reasoning: if option["currentValue"].as_str() == Some(id) {
                        default_reasoning.into()
                    } else {
                        String::new()
                    },
                    reasoning_efforts: if option["currentValue"].as_str() == Some(id) {
                        reasoning_efforts.clone()
                    } else {
                        Vec::new()
                    },
                })
            })
            .collect();
    }
    state["models"]["availableModels"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let id = entry["modelId"].as_str()?;
            Some(BackendModel {
                id: id.into(),
                name: entry["name"].as_str().unwrap_or(id).into(),
                is_default: state["models"]["currentModelId"].as_str() == Some(id),
                default_reasoning: if state["models"]["currentModelId"].as_str() == Some(id) {
                    default_reasoning.into()
                } else {
                    String::new()
                },
                reasoning_efforts: if state["models"]["currentModelId"].as_str() == Some(id) {
                    reasoning_efforts.clone()
                } else {
                    Vec::new()
                },
            })
        })
        .collect()
}

pub(crate) fn usage(value: &Value) -> Option<TokenUsage> {
    let value = value.get("usage").unwrap_or(value);
    let read = |names: &[&str]| {
        names
            .iter()
            .find_map(|name| value[*name].as_u64())
            .map(|n| n.min(u32::MAX as u64) as u32)
    };
    let cached = read(&["cachedReadTokens", "cachedContentTokenCount"]).unwrap_or_default();
    let input = read(&["inputTokens"])
        .map(|n| n.saturating_add(cached))
        .or_else(|| read(&["promptTokens", "input_tokens"]));
    let output = read(&["outputTokens", "completionTokens", "output_tokens"]);
    let total = read(&["totalTokens", "total_tokens"]);
    if input.is_none() && output.is_none() && total.is_none() {
        return None;
    }
    Some(
        TokenUsage::new(
            input.unwrap_or_default(),
            output.unwrap_or_default(),
            total.unwrap_or_else(|| {
                input
                    .unwrap_or_default()
                    .saturating_add(output.unwrap_or_default())
            }),
        )
        .with_cached_prompt_tokens(cached),
    )
}

pub(crate) fn update_events(update: &Value) -> Vec<BackendEvent> {
    let mut events = Vec::new();
    match update["sessionUpdate"].as_str().unwrap_or("") {
        "agent_message_chunk" => events.push(BackendEvent::Text(text(&update["content"]))),
        "tool_call" | "tool_call_update" => {
            let id = update["toolCallId"].as_str().unwrap_or("").to_owned();
            let label = update["title"]
                .as_str()
                .unwrap_or("Antigravity tool")
                .to_owned();
            match update["status"].as_str().unwrap_or("pending") {
                "completed" | "failed" => events.push(BackendEvent::ToolFinished {
                    id: id.clone(),
                    label,
                    summary: text(&update["content"]),
                }),
                _ if update["sessionUpdate"] == "tool_call" => {
                    events.push(BackendEvent::ToolStarted {
                        id: id.clone(),
                        label,
                    })
                }
                _ => {}
            }
            let output = text(&update["content"]);
            if !output.is_empty() {
                events.push(BackendEvent::ToolOutput { id, text: output });
            }
            for location in update["locations"].as_array().into_iter().flatten() {
                if matches!(update["kind"].as_str(), Some("edit" | "delete" | "move"))
                    && update["status"] == "completed"
                    && let Some(path) = location["path"].as_str()
                {
                    events.push(BackendEvent::FileChanged {
                        path: path.into(),
                        change_type: update["kind"].as_str().unwrap_or("edit").into(),
                    });
                }
            }
        }
        "usage_update" => {
            if let Some(usage) = usage(update) {
                events.push(BackendEvent::Usage(usage));
            }
            events.push(BackendEvent::ProviderMetadata(update.clone()));
        }
        "agent_thought_chunk" | "plan" => {
            events.push(BackendEvent::ProviderMetadata(update.clone()));
        }
        _ => {}
    }
    events
}
