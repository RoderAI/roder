use super::*;

pub(super) fn response_input_items_with_options(
    request: &AgentInferenceRequest,
    tool_name_map: &ResponsesToolNameMap,
    profile: ResponsesProviderProfile,
    supports_images: bool,
) -> Vec<Value> {
    let mut items = response_input_items(request, tool_name_map, profile, supports_images);
    if matches!(
        profile,
        ResponsesProviderProfile::OpenRouter | ResponsesProviderProfile::Fireworks
    ) {
        // These profiles do not use top-level `instructions`; fold the full
        // InstructionBundle into leading system-role input messages.
        let mut instruction_items = Vec::new();
        if let Some(system) = request
            .instructions
            .system
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            instruction_items.push(system_input_message(system));
        }
        if let Some(developer) = request
            .instructions
            .developer
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            instruction_items.push(system_input_message(&format!(
                "Developer instructions:\n{developer}"
            )));
        }
        if let Some(context) = request
            .instructions
            .developer_context
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            instruction_items.push(developer_context_input_message(context));
        }
        if !instruction_items.is_empty() {
            instruction_items.extend(items);
            items = instruction_items;
        }
    } else if let Some(context) = request
        .instructions
        .developer_context
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        // Keep per-turn developer_context out of the stable top-level
        // `instructions` prefix so prompt-cache breakpoints survive. Render it
        // as a leading input message after instructions/system+developer.
        let mut with_context = vec![developer_context_input_message(context)];
        with_context.extend(items);
        items = with_context;
    }
    items
}

/// Stable system + developer text for the Responses top-level `instructions`
/// field (OpenAI, Codex, xAI, SuperGrok). Omits per-turn `developer_context`.
pub(super) fn stable_responses_instructions(request: &AgentInferenceRequest) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(system) = request
        .instructions
        .system
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        parts.push(system.to_string());
    }
    if let Some(developer) = request
        .instructions
        .developer
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        parts.push(format!("Developer instructions:\n{developer}"));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

pub(super) fn system_input_message(text: &str) -> Value {
    json!({
        "type": "message",
        "role": "system",
        "content": [{ "type": "input_text", "text": text }]
    })
}

pub(super) fn developer_context_input_message(context: &str) -> Value {
    system_input_message(&format!("Developer context (this turn):\n{context}"))
}

pub(super) fn user_message_content(
    message: &roder_api::transcript::UserMessage,
    supports_images: bool,
) -> Vec<Value> {
    let mut content = Vec::new();
    if !message.text.is_empty() {
        content.push(json!({ "type": "input_text", "text": message.text }));
    }
    if supports_images {
        content.extend(message.images.iter().map(|image| {
            json!({
                "type": "input_image",
                "image_url": image.image_url,
            })
        }));
    }
    if content.is_empty() {
        content.push(json!({ "type": "input_text", "text": "" }));
    }
    content
}
