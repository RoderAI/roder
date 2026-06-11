use futures::StreamExt;
use roder_api::inference::{
    AgentInferenceRequest, InferenceEngine, InferenceEvent, InferenceTurnContext,
    InstructionBundle, ModelSelection, OutputConfig, ReasoningConfig, RuntimeHints,
};
use roder_api::tools::ToolChoice;
use roder_api::transcript::{TranscriptItem, UserMessage};
use roder_ext_anthropic::AnthropicEngine;

#[tokio::test]
#[ignore = "requires RODER_ANTHROPIC_LIVE=1 and ANTHROPIC_API_KEY"]
async fn live_anthropic_streaming_emits_incremental_message_deltas() {
    if std::env::var("RODER_ANTHROPIC_LIVE").ok().as_deref() != Some("1") {
        eprintln!("set RODER_ANTHROPIC_LIVE=1 to run live Anthropic streaming tests");
        return;
    }
    let api_key = std::env::var("ANTHROPIC_API_KEY").expect("ANTHROPIC_API_KEY must be set");

    let engine = AnthropicEngine::new(api_key);
    let request = AgentInferenceRequest {
        model: ModelSelection {
            provider: "anthropic".to_string(),
            model: "claude-haiku-4-5-20251001".to_string(),
        },
        instructions: InstructionBundle::default(),
        transcript: vec![TranscriptItem::UserMessage(UserMessage::text(
            "Write five numbered one-sentence facts about the ocean.",
        ))],
        tools: Vec::new(),
        tool_choice: ToolChoice::Auto,
        reasoning: ReasoningConfig::default(),
        output: OutputConfig {
            max_tokens: Some(300),
            ..OutputConfig::default()
        },
        runtime: RuntimeHints::default(),
        metadata: serde_json::json!({}),
    };

    let mut stream = engine
        .stream_turn(
            InferenceTurnContext {
                thread_id: "live-test",
                turn_id: "live-turn",
                tool_executor: None,
            },
            request,
        )
        .await
        .unwrap();

    let mut message_deltas = 0;
    let mut deltas_before_completed = 0;
    let mut saw_usage = false;
    let mut completed = false;
    let mut stop_reason = None;
    while let Some(event) = stream.next().await {
        match event.unwrap() {
            InferenceEvent::MessageDelta(_) => {
                message_deltas += 1;
                if !completed {
                    deltas_before_completed += 1;
                }
            }
            InferenceEvent::Usage(usage) => {
                saw_usage = true;
                assert!(usage.prompt_tokens > 0);
                assert!(usage.completion_tokens > 0);
            }
            InferenceEvent::Completed(metadata) => {
                completed = true;
                stop_reason = metadata.stop_reason;
            }
            _ => {}
        }
    }
    eprintln!(
        "live streaming: {message_deltas} MessageDelta events ({deltas_before_completed} before Completed)"
    );

    // The API coalesces deltas; more than one proves the reply streamed
    // incrementally instead of arriving as a single blob.
    assert!(
        deltas_before_completed > 1,
        "expected >1 incremental text deltas before Completed, got {deltas_before_completed}"
    );
    assert!(completed, "stream ended without a Completed event");
    assert!(saw_usage);
    assert_eq!(stop_reason.as_deref(), Some("end_turn"));
}

#[tokio::test]
#[ignore = "requires RODER_ANTHROPIC_LIVE=1 and ANTHROPIC_API_KEY"]
async fn live_anthropic_tools_smoke_is_explicitly_opt_in() {
    if std::env::var("RODER_ANTHROPIC_LIVE").ok().as_deref() != Some("1") {
        eprintln!("set RODER_ANTHROPIC_LIVE=1 to run live Anthropic tool smoke tests");
        return;
    }

    let has_key = std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .is_some();

    assert!(
        has_key,
        "live Anthropic tool smoke tests require ANTHROPIC_API_KEY"
    );
}

/**
 * Opt-in live validation that the provider-native tool-search request body
 * produced by the real request mapper is accepted by the Anthropic Messages
 * API (dated `tool_search_tool_*` entry plus `defer_loading` tools).
 *
 * Set `RODER_ANTHROPIC_TOOL_SEARCH_BETA` if the live API requires an
 * `anthropic-beta` header for the tool-search variants.
 */
#[tokio::test]
#[ignore = "requires RODER_ANTHROPIC_TOOL_SEARCH_LIVE=1 and ANTHROPIC_API_KEY"]
async fn live_anthropic_tool_search_smoke_is_explicitly_opt_in() {
    if std::env::var("RODER_ANTHROPIC_TOOL_SEARCH_LIVE")
        .ok()
        .as_deref()
        != Some("1")
    {
        eprintln!("set RODER_ANTHROPIC_TOOL_SEARCH_LIVE=1 to run live Anthropic tool-search tests");
        return;
    }

    let api_key = std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .expect("live Anthropic tool-search smoke tests require ANTHROPIC_API_KEY");
    let model = std::env::var("RODER_ANTHROPIC_TOOL_SEARCH_MODEL")
        .unwrap_or_else(|_| "claude-fable-5".to_string());

    let mut request = tool_search_request(&model);
    request.runtime.tool_search =
        roder_api::inference::ToolSearchConfig::provider_native();
    let mut body = roder_ext_anthropic::AnthropicEngine::map_request(&request);
    let tools = body["tools"].as_array().cloned().unwrap_or_default();
    assert_eq!(
        tools.len(),
        3,
        "expected the tool_search entry plus two deferred tools: {body}"
    );
    assert!(
        tools[0]["type"]
            .as_str()
            .is_some_and(|kind| kind.starts_with("tool_search_tool_")),
        "expected dated tool-search entry first: {body}"
    );
    body["stream"] = serde_json::json!(false);

    let mut builder = reqwest::Client::new()
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01");
    if let Ok(beta) = std::env::var("RODER_ANTHROPIC_TOOL_SEARCH_BETA")
        && !beta.trim().is_empty()
    {
        builder = builder.header("anthropic-beta", beta);
    }
    let response = builder
        .json(&body)
        .send()
        .await
        .expect("send live Anthropic tool-search request");
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    assert!(
        status.is_success(),
        "live Anthropic tool-search request rejected ({status}): {text}"
    );
}

fn tool_search_request(model: &str) -> roder_api::inference::AgentInferenceRequest {
    use roder_api::inference::{
        InstructionBundle, ModelSelection, OutputConfig, ReasoningConfig, RuntimeHints,
    };
    use roder_api::tools::{ToolChoice, ToolSpec};
    use roder_api::transcript::{TranscriptItem, UserMessage};

    roder_api::inference::AgentInferenceRequest {
        model: ModelSelection {
            provider: "anthropic".to_string(),
            model: model.to_string(),
        },
        instructions: InstructionBundle {
            system: Some("You are a coding agent; search for the right tool.".to_string()),
            developer: None,
            developer_context: None,
        },
        transcript: vec![TranscriptItem::UserMessage(UserMessage::text(
            "Which tool would read README.md? Answer in one sentence without calling tools.",
        ))],
        tools: vec![
            ToolSpec {
                name: "read_file".to_string(),
                description: "Read a file from the workspace.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                    "required": ["path"]
                }),
            },
            ToolSpec {
                name: "edit_file".to_string(),
                description: "Edit a file by replacing an exact string.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                    "required": ["path"]
                }),
            },
        ],
        tool_choice: ToolChoice::Auto,
        reasoning: ReasoningConfig {
            enabled: false,
            level: None,
        },
        output: OutputConfig {
            max_tokens: Some(128),
            temperature: None,
            top_p: None,
            response_format: None,
        },
        runtime: RuntimeHints::default(),
        metadata: serde_json::json!({}),
    }
}
