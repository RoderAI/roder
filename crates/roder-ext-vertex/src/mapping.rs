//! Request/response mapping for Gemini-on-Vertex. The body shapes are the
//! Gemini API's (`contents`/`systemInstruction`/`generationConfig`/`tools`),
//! reused from `roder-ext-gemini`; only auth and endpoint shape differ.

use std::collections::{HashMap, HashSet};

use roder_api::inference::{AgentInferenceRequest, TokenUsage, ToolCallCompleted};
use serde_json::{Value, json};

use crate::media::vertex_user_message_parts;
use crate::schema::vertex_schema;

pub(crate) fn map_request(request: &AgentInferenceRequest) -> anyhow::Result<Value> {
    let mut body = json!({
        "contents": vertex_contents(request)?,
    });
    let mut system_parts = Vec::new();
    if let Some(text) = request.instructions.system.as_deref() {
        system_parts.push(json!({ "text": text }));
    }
    if let Some(text) = request.instructions.developer.as_deref() {
        system_parts.push(json!({ "text": text }));
    }
    if !system_parts.is_empty() {
        body["systemInstruction"] = json!({ "parts": system_parts });
    }
    let generation_config = vertex_generation_config(request);
    if !generation_config
        .as_object()
        .is_some_and(|object| object.is_empty())
    {
        body["generationConfig"] = generation_config;
    }
    if !request.tools.is_empty() {
        body["tools"] = json!([{
            "functionDeclarations": request
                .tools
                .iter()
                .map(|tool| {
                    let tool = tool.normalized_for_model(roder_api::ToolSchemaPolicy::warning());
                    json!({
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": vertex_schema(tool.parameters),
                    })
                })
                .collect::<Vec<_>>()
        }]);
        body["toolConfig"] = vertex_tool_config(&request.tool_choice);
    }
    Ok(body)
}

fn vertex_contents(request: &AgentInferenceRequest) -> anyhow::Result<Vec<Value>> {
    let mut contents = Vec::new();
    let mut replay = VertexProviderReplay::default();
    for item in &request.transcript {
        match item {
            roder_api::transcript::TranscriptItem::UserMessage(message) => {
                contents.push(json!({
                    "role": "user",
                    "parts": vertex_user_message_parts(message)?
                }));
            }
            roder_api::transcript::TranscriptItem::AssistantMessage(message) => {
                contents.push(json!({
                    "role": "model",
                    "parts": [{ "text": message.text }]
                }));
            }
            roder_api::transcript::TranscriptItem::ToolCall(call) => {
                if replay.should_skip_tool_call(call) {
                    continue;
                }
                contents.push(json!({
                    "role": "model",
                    "parts": [{
                        "functionCall": {
                            "id": call.id,
                            "name": call.name,
                            "args": parse_json_object(&call.arguments)
                        }
                    }]
                }));
            }
            roder_api::transcript::TranscriptItem::ToolResult(result) => {
                contents.push(json!({
                    "role": "user",
                    "parts": [{
                        "functionResponse": {
                            "id": result.id,
                            "name": result.name.clone().unwrap_or_default(),
                            "response": { "result": result.result, "is_error": result.is_error }
                        }
                    }]
                }));
            }
            roder_api::transcript::TranscriptItem::ProviderMetadata(metadata) => {
                append_provider_function_call_content(metadata, &mut contents, &mut replay);
            }
            _ => {}
        }
    }
    Ok(contents)
}

/**
 * Replays provider-emitted function-call content (with thought signatures)
 * from ProviderMetadata transcript items and dedupes the matching canonical
 * ToolCall records so each call is sent once.
 */
#[derive(Default)]
struct VertexProviderReplay {
    call_ids: HashSet<String>,
    call_keys: HashMap<(String, String), usize>,
}

impl VertexProviderReplay {
    fn record_part(&mut self, part: &Value) {
        let Some(call) = part.get("functionCall") else {
            return;
        };
        if let Some(id) = call.get("id").and_then(Value::as_str)
            && !id.is_empty()
        {
            self.call_ids.insert(id.to_string());
        }
        if let Some(name) = call.get("name").and_then(Value::as_str) {
            let args = call.get("args").cloned().unwrap_or_else(|| json!({}));
            *self
                .call_keys
                .entry((name.to_string(), args.to_string()))
                .or_default() += 1;
        }
    }

    fn should_skip_tool_call(&mut self, call: &roder_api::transcript::ToolCallRecord) -> bool {
        if !call.id.is_empty() && self.call_ids.remove(&call.id) {
            return true;
        }
        let key = (call.name.clone(), canonical_tool_arguments(&call.arguments));
        let Some(count) = self.call_keys.get_mut(&key) else {
            return false;
        };
        *count = count.saturating_sub(1);
        if *count == 0 {
            self.call_keys.remove(&key);
        }
        true
    }
}

fn append_provider_function_call_content(
    metadata: &Value,
    contents: &mut Vec<Value>,
    replay: &mut VertexProviderReplay,
) {
    let Some(content) = metadata.pointer("/candidates/0/content") else {
        return;
    };
    let Some(parts) = content.get("parts").and_then(Value::as_array) else {
        return;
    };
    if !parts.iter().any(|part| part.get("functionCall").is_some()) {
        return;
    }
    for part in parts {
        replay.record_part(part);
    }
    let mut content = content.clone();
    if content.get("role").is_none() {
        content["role"] = json!("model");
    }
    contents.push(content);
}

fn canonical_tool_arguments(raw: &str) -> String {
    serde_json::from_str::<Value>(raw)
        .ok()
        .unwrap_or_else(|| json!({}))
        .to_string()
}

fn vertex_tool_config(choice: &roder_api::tools::ToolChoice) -> Value {
    let mode = match choice {
        roder_api::tools::ToolChoice::Auto => "AUTO",
        roder_api::tools::ToolChoice::Any => "ANY",
        roder_api::tools::ToolChoice::None => "NONE",
        roder_api::tools::ToolChoice::Specific(_) => "ANY",
    };
    let mut function_calling_config = json!({ "mode": mode });
    if let roder_api::tools::ToolChoice::Specific(name) = choice {
        function_calling_config["allowedFunctionNames"] = json!([name]);
    }
    json!({ "functionCallingConfig": function_calling_config })
}

fn vertex_generation_config(request: &AgentInferenceRequest) -> Value {
    let mut config = json!({});
    if let Some(max_tokens) = request.output.max_tokens {
        config["maxOutputTokens"] = json!(max_tokens);
    }
    if let Some(temperature) = request.output.temperature {
        config["temperature"] = json!(temperature);
    }
    if let Some(top_p) = request.output.top_p {
        config["topP"] = json!(top_p);
    }
    if let Some(response_format) = request.output.response_format.as_ref() {
        if let Some(mime_type) = response_format.get("mime_type").and_then(|v| v.as_str()) {
            config["responseMimeType"] = json!(mime_type);
        } else if response_format.get("type").and_then(|v| v.as_str()) == Some("json_object") {
            config["responseMimeType"] = json!("application/json");
        }
        if let Some(schema) = response_format.get("schema") {
            config["responseSchema"] = schema.clone();
        }
    }
    if request.reasoning.enabled
        && let Some(thinking_config) = vertex_thinking_config(request.reasoning.level.as_deref())
    {
        config["thinkingConfig"] = thinking_config;
    }
    config
}

fn vertex_thinking_config(level: Option<&str>) -> Option<Value> {
    match level {
        Some("minimal") => Some(json!({ "thinkingLevel": "MINIMAL" })),
        Some("low") => Some(json!({ "thinkingLevel": "LOW" })),
        Some("medium") => Some(json!({ "thinkingLevel": "MEDIUM" })),
        Some("high") => Some(json!({ "thinkingLevel": "HIGH" })),
        None => None,
        Some(_) => None,
    }
}

fn parse_json_object(raw: &str) -> Value {
    serde_json::from_str::<Value>(raw)
        .ok()
        .filter(|value| value.is_object())
        .unwrap_or_else(|| json!({}))
}

/// True when the part is reasoning: either `thought: true` alongside `text`,
/// or a direct `thought` string.
pub(crate) fn part_thought_text(part: &Value) -> Option<&str> {
    let thought = part.get("thought")?;
    if thought.as_bool().unwrap_or(false) {
        return part.get("text").and_then(Value::as_str);
    }
    thought.as_str()
}

pub(crate) fn part_message_text(part: &Value) -> Option<&str> {
    if part
        .get("thought")
        .is_some_and(|thought| thought.as_bool().unwrap_or(false) || thought.as_str().is_some())
    {
        return None;
    }
    part.get("text").and_then(Value::as_str)
}

pub(crate) fn part_tool_call(part: &Value) -> Option<ToolCallCompleted> {
    let call = part.get("functionCall")?;
    let name = call.get("name").and_then(Value::as_str)?.to_string();
    Some(ToolCallCompleted {
        id: call
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        name,
        arguments: call
            .get("args")
            .map(Value::to_string)
            .unwrap_or_else(|| "{}".to_string()),
    })
}

pub(crate) fn extract_usage(usage: &Value) -> TokenUsage {
    TokenUsage::new(
        number_to_u32(usage.get("promptTokenCount")).unwrap_or_default(),
        number_to_u32(usage.get("candidatesTokenCount")).unwrap_or_default(),
        number_to_u32(usage.get("totalTokenCount")).unwrap_or_default(),
    )
    .with_cached_prompt_tokens(
        number_to_u32(usage.get("cachedContentTokenCount")).unwrap_or_default(),
    )
}

/**
 * Maps Vertex finish reasons onto the lowercase stop reasons that
 * `roder_api::inference::finish_reason_from_stop_reason` canonicalizes.
 * `STOP` on a turn that produced function calls is a tool-use stop. Unknown
 * reasons pass through unchanged.
 */
pub(crate) fn canonical_stop_reason(finish_reason: &str, has_tool_calls: bool) -> String {
    match finish_reason {
        "STOP" if has_tool_calls => "tool_use".to_string(),
        "STOP" => "stop".to_string(),
        "MAX_TOKENS" => "max_tokens".to_string(),
        "SAFETY" | "RECITATION" | "BLOCKLIST" | "PROHIBITED_CONTENT" | "SPII" => {
            "content_filter".to_string()
        }
        other => other.to_string(),
    }
}

fn number_to_u32(value: Option<&Value>) -> Option<u32> {
    value?.as_u64().and_then(|n| u32::try_from(n).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::inference::{
        InstructionBundle, ModelSelection, OutputConfig, ReasoningConfig, RuntimeHints,
    };
    use roder_api::tools::{ToolChoice, ToolSpec};
    use roder_api::transcript::{
        AssistantMessage, ToolCallRecord, ToolResultRecord, TranscriptItem, UserMessage,
    };

    pub(crate) fn request() -> AgentInferenceRequest {
        AgentInferenceRequest {
            model: ModelSelection {
                provider: "vertex".to_string(),
                model: "gemini-3.5-flash".to_string(),
            },
            instructions: InstructionBundle {
                system: Some("system".to_string()),
                developer: Some("developer".to_string()),
                developer_context: None,
            },
            transcript: vec![
                TranscriptItem::UserMessage(UserMessage::text("Hello")),
                TranscriptItem::AssistantMessage(AssistantMessage {
                    text: "Hi".to_string(),
                    phase: None,
                }),
                TranscriptItem::ToolCall(ToolCallRecord {
                    id: "call_1".to_string(),
                    name: "shell".to_string(),
                    arguments: r#"{"cmd":"pwd"}"#.to_string(),
                }),
                TranscriptItem::ToolResult(ToolResultRecord {
                    id: "call_1".to_string(),
                    name: Some("shell".to_string()),
                    result: "ok".to_string(),
                    display_payload: None,
                    is_error: false,
                }),
            ],
            tools: vec![ToolSpec {
                name: "shell".to_string(),
                description: "Run a shell command".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": { "cmd": { "type": "string" } },
                    "required": ["cmd"]
                }),
            }],
            tool_choice: ToolChoice::Specific("shell".to_string()),
            reasoning: ReasoningConfig {
                enabled: true,
                level: Some("medium".to_string()),
            },
            output: OutputConfig {
                max_tokens: Some(128),
                temperature: Some(0.4),
                top_p: Some(0.95),
                response_format: Some(json!({
                    "type": "json_object",
                    "schema": { "type": "object" }
                })),
            },
            runtime: RuntimeHints::default(),
            metadata: json!({}),
        }
    }

    #[test]
    fn maps_request_to_gemini_wire_shape() {
        let body = map_request(&request()).unwrap();

        assert_eq!(body["systemInstruction"]["parts"][0]["text"], "system");
        assert_eq!(body["systemInstruction"]["parts"][1]["text"], "developer");
        assert_eq!(body["contents"][0]["role"], "user");
        assert_eq!(body["contents"][1]["role"], "model");
        assert_eq!(
            body["contents"][2]["parts"][0]["functionCall"]["name"],
            "shell"
        );
        assert_eq!(
            body["contents"][3]["parts"][0]["functionResponse"]["response"]["result"],
            "ok"
        );
        assert_eq!(body["generationConfig"]["maxOutputTokens"], 128);
        assert_eq!(
            body["generationConfig"]["responseMimeType"],
            "application/json"
        );
        assert_eq!(
            body["generationConfig"]["thinkingConfig"]["thinkingLevel"],
            "MEDIUM"
        );
        assert_eq!(body["tools"][0]["functionDeclarations"][0]["name"], "shell");
        assert_eq!(
            body["toolConfig"]["functionCallingConfig"]["allowedFunctionNames"][0],
            "shell"
        );
        // The model is addressed in the URL path on Vertex, never in the body.
        assert!(body.get("model").is_none());
    }

    #[test]
    fn replays_function_call_content_with_thought_signature_exactly_once() {
        let mut request = request();
        request.transcript = vec![
            TranscriptItem::UserMessage(UserMessage::text("List files")),
            TranscriptItem::ProviderMetadata(json!({
                "candidates": [{
                    "content": {
                        "role": "model",
                        "parts": [{
                            "functionCall": {
                                "name": "shell",
                                "args": { "cmd": "ls" }
                            },
                            "thoughtSignature": "signed-thought"
                        }]
                    }
                }]
            })),
            TranscriptItem::ToolCall(ToolCallRecord {
                id: "call_1".to_string(),
                name: "shell".to_string(),
                arguments: r#"{"cmd":"ls"}"#.to_string(),
            }),
            TranscriptItem::ToolResult(ToolResultRecord {
                id: "call_1".to_string(),
                name: Some("shell".to_string()),
                result: "Cargo.toml".to_string(),
                display_payload: None,
                is_error: false,
            }),
        ];

        let body = map_request(&request).unwrap();
        let contents = body["contents"].as_array().unwrap();
        let function_call_count = contents
            .iter()
            .filter(|content| {
                content
                    .pointer("/parts/0/functionCall/name")
                    .and_then(Value::as_str)
                    == Some("shell")
            })
            .count();

        assert_eq!(function_call_count, 1);
        assert_eq!(
            contents[1]["parts"][0]["thoughtSignature"],
            "signed-thought"
        );
        assert_eq!(contents[2]["parts"][0]["functionResponse"]["name"], "shell");
    }

    #[test]
    fn normalizes_tool_schema_for_vertex_tools() {
        let mut request = request();
        request.tools = vec![ToolSpec {
            name: "shell".to_string(),
            description: "Run shell command".to_string(),
            parameters: json!({
                "type": "object",
                "properties": { "command": { "type": "string" } },
                "additionalProperties": false,
                "required": ["command"]
            }),
        }];

        let body = map_request(&request).unwrap();
        let schema =
            serde_json::to_string(&body["tools"][0]["functionDeclarations"][0]["parameters"])
                .unwrap();

        assert!(
            schema.starts_with(r#"{"type":"object","required":["command"],"properties":"#),
            "{schema}"
        );
    }

    #[test]
    fn rejects_unsupported_thinking_levels() {
        assert_eq!(vertex_thinking_config(Some("xhigh")), None);
        assert_eq!(vertex_thinking_config(Some("none")), None);
        assert_eq!(vertex_thinking_config(None), None);
    }

    #[test]
    fn splits_parts_into_text_thought_and_tool_calls() {
        let text_part = json!({ "text": "hello" });
        let flagged_thought = json!({ "text": "thinking...", "thought": true });
        let string_thought = json!({ "thought": "direct thought" });
        let call_part = json!({
            "functionCall": { "id": "call_2", "name": "shell", "args": { "cmd": "ls" } }
        });

        assert_eq!(part_message_text(&text_part), Some("hello"));
        assert_eq!(part_message_text(&flagged_thought), None);
        assert_eq!(part_thought_text(&flagged_thought), Some("thinking..."));
        assert_eq!(part_thought_text(&string_thought), Some("direct thought"));
        assert_eq!(
            part_tool_call(&call_part),
            Some(ToolCallCompleted {
                id: "call_2".to_string(),
                name: "shell".to_string(),
                arguments: r#"{"cmd":"ls"}"#.to_string(),
            })
        );
    }

    #[test]
    fn extracts_usage_including_cached_tokens() {
        let usage = extract_usage(&json!({
            "promptTokenCount": 10,
            "candidatesTokenCount": 3,
            "totalTokenCount": 13,
            "cachedContentTokenCount": 9
        }));

        assert_eq!(
            usage,
            TokenUsage::new(10, 3, 13).with_cached_prompt_tokens(9)
        );
    }

    #[test]
    fn maps_finish_reasons_to_canonical_stop_reasons() {
        assert_eq!(canonical_stop_reason("STOP", false), "stop");
        assert_eq!(canonical_stop_reason("STOP", true), "tool_use");
        assert_eq!(canonical_stop_reason("MAX_TOKENS", false), "max_tokens");
        for filtered in [
            "SAFETY",
            "RECITATION",
            "BLOCKLIST",
            "PROHIBITED_CONTENT",
            "SPII",
        ] {
            assert_eq!(canonical_stop_reason(filtered, false), "content_filter");
        }
        assert_eq!(
            canonical_stop_reason("MALFORMED_FUNCTION_CALL", false),
            "MALFORMED_FUNCTION_CALL"
        );
    }
}
