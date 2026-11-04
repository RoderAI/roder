use roder_api::transcript::{ToolResultRecord, UserMessage, tool_result_image};
use serde_json::{Value, json};

/// Keep text-only requests as strings for endpoints without multimodal support.
pub(crate) fn user_content(message: &UserMessage) -> Value {
    if message.images.is_empty() {
        return json!(message.text);
    }
    let mut parts = Vec::new();
    if !message.text.is_empty() {
        parts.push(json!({ "type": "text", "text": message.text }));
    }
    parts.extend(
        message
            .images
            .iter()
            .map(|image| json!({ "type": "image_url", "image_url": { "url": image.image_url } })),
    );
    Value::Array(parts)
}

/// Chat Completions tool messages cannot contain image parts. Replay validated
/// inline images as a user message after the entire batch of tool responses.
pub(crate) fn tool_image_message(result: &ToolResultRecord) -> Option<Value> {
    let (mime, data) = tool_result_image(result.display_payload.as_ref())?;
    Some(json!({
        "role": "user",
        "content": [
            { "type": "text", "text": format!("Image from tool call {}:", result.id) },
            { "type": "image_url", "image_url": { "url": format!("data:{mime};base64,{data}") } }
        ]
    }))
}
