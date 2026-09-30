//! Adapt native Responses computer items to Roder's local tool dispatcher.
use super::*;
use roder_api::computer::COMPUTER_TOOL_NAME;
use roder_api::transcript::{ToolResultRecord, TranscriptItem};

pub(super) fn computer_call(item: &Value) -> Option<ToolCallCompleted> {
    if item["type"] != "computer_call" || item["status"] != "completed" {
        return None;
    }
    Some(ToolCallCompleted {
        id: item["call_id"].as_str()?.to_string(),
        name: COMPUTER_TOOL_NAME.into(),
        // The typed executor validates the entire shape; malformed actions
        // are never converted to another tool or silently executed partially.
        arguments: json!({"actions": item.get("actions").cloned().unwrap_or(Value::Null)})
            .to_string(),
    })
}

pub(super) fn started_computer_call(item: &Value) -> Option<ToolCallStarted> {
    (item["type"] == "computer_call")
        .then(|| ToolCallStarted {
            id: item["call_id"].as_str().unwrap_or_default().to_string(),
            name: COMPUTER_TOOL_NAME.into(),
        })
        .filter(|call| !call.id.is_empty())
}

pub(super) fn computer_call_ids(transcript: &[TranscriptItem]) -> HashSet<String> {
    let mut ids: HashSet<_> = transcript
        .iter()
        .filter_map(|item| match item {
            TranscriptItem::ToolCall(call) if call.name == COMPUTER_TOOL_NAME => {
                Some(call.id.clone())
            }
            _ => None,
        })
        .collect();
    ids.extend(
        raw_output(transcript)
            .filter(|item| item["type"] == "computer_call")
            .filter_map(|item| item["call_id"].as_str().map(str::to_string)),
    );
    ids
}

pub(super) fn computer_output(result: &ToolResultRecord) -> Option<Value> {
    let image = tool_output_image_block(result)?;
    if image["image_url"].as_str().is_none_or(str::is_empty) {
        return None;
    }
    Some(json!({"type":"computer_call_output","call_id":result.id,
        "output":{"type":"computer_screenshot","image_url":image["image_url"],"detail":"original"}}))
}

pub(super) fn validate_computer_request(
    request: &AgentInferenceRequest,
    profile: ResponsesProviderProfile,
) -> anyhow::Result<()> {
    let advertised = request
        .tools
        .iter()
        .any(|tool| tool.name == COMPUTER_TOOL_NAME);
    let ids = computer_call_ids(&request.transcript);
    ensure_computer_profile(advertised || !ids.is_empty(), profile)?;
    anyhow::ensure!(
        !(advertised || !ids.is_empty()) || request.model.provider == PROVIDER_OPENAI,
        "native computer is supported by the OpenAI API provider; the signed-in Codex endpoint does not support this tool"
    );
    for item in &request.transcript {
        if let TranscriptItem::ToolResult(result) = item
            && ids.contains(&result.id)
            && computer_output(result).is_none()
        {
            anyhow::bail!(
                "native computer call {} has no screenshot: {}; restore its browser session and capture an observation before continuing",
                result.id,
                result.result
            );
        }
    }
    Ok(())
}

fn ensure_computer_profile(enabled: bool, profile: ResponsesProviderProfile) -> anyhow::Result<()> {
    anyhow::ensure!(
        !enabled || profile == ResponsesProviderProfile::OpenAi,
        "native computer requires an OpenAI Responses provider; it cannot be sent as a function tool"
    );
    Ok(())
}

pub(super) fn computer_protocol_error(kind: &str, data: &Value) -> Option<String> {
    let items: Vec<&Value> = match kind {
        "response.output_item.done" => data.get("item").into_iter().collect(),
        "response.completed" => data
            .pointer("/response/output")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .collect(),
        _ => return None,
    };
    items.into_iter().find(|item| item["type"] == "computer_call" &&
        (item["status"] != "completed" || item["call_id"].as_str().is_none_or(str::is_empty)
        || !item["actions"].is_array())).map(|_| "invalid native computer_call: requires completed status, nonempty call_id and actions array".into())
}
