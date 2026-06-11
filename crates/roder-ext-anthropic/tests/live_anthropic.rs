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

    let engine = AnthropicEngine::new(Some(api_key));
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
    request.runtime.tool_search = roder_api::inference::ToolSearchConfig::provider_native();
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

/**
 * Opt-in live validation of the per-turn developer-context mapping on models
 * with mid-conversation system-message support: the mapped body (trailing
 * `role: system` message + explicit breakpoint on the last stable message
 * block) is accepted by the live API, and a warm turn with a CHANGED
 * developer context still reads the conversation tier from cache — the
 * regression this mapping exists to avoid is the history re-processing on
 * every turn-start that trailing system BLOCKS cause.
 */
#[tokio::test]
#[ignore = "requires RODER_ANTHROPIC_LIVE=1 and ANTHROPIC_API_KEY"]
async fn live_anthropic_system_role_developer_context_preserves_conversation_cache() {
    if std::env::var("RODER_ANTHROPIC_LIVE").ok().as_deref() != Some("1") {
        eprintln!("set RODER_ANTHROPIC_LIVE=1 to run live Anthropic system-role context tests");
        return;
    }
    let api_key = std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .expect("live Anthropic system-role context tests require ANTHROPIC_API_KEY");
    let model = std::env::var("RODER_ANTHROPIC_SYSTEM_ROLE_MODEL")
        .unwrap_or_else(|_| "claude-opus-4-8".to_string());

    let pad = |label: &str, n: usize| {
        (0..n)
            .map(|i| format!("{label}-filler-{i}"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    // Above the model's minimum cacheable prefix (4096 tokens on Opus-tier);
    // the nonce makes turn 1 a true cold start.
    let nonce = uuid_like();
    let mut request = tool_search_request(&model);
    request.tools = Vec::new();
    request.instructions.system = Some(format!(
        "Harness system prompt, run {nonce}. Answer with one short sentence. {}",
        pad("sys", 1200)
    ));
    request.transcript = vec![TranscriptItem::UserMessage(UserMessage::text(format!(
        "Turn 1: say OK. Context dump: {}",
        pad("hist", 800)
    )))];
    request.instructions.developer_context =
        Some("Session context: connected account A-1, turn 1.".to_string());

    let post = |body: serde_json::Value, api_key: String| async move {
        let response = reqwest::Client::new()
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .header("anthropic-beta", "mid-conversation-system-2026-04-07")
            .json(&body)
            .send()
            .await
            .expect("send live Anthropic system-role context request");
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        assert!(
            status.is_success(),
            "live system-role context request rejected ({status}): {text}"
        );
        serde_json::from_str::<serde_json::Value>(&text).expect("parse response")
    };

    let mut body = roder_ext_anthropic::AnthropicEngine::map_request(&request);
    assert_eq!(
        body["messages"].as_array().unwrap().last().unwrap()["role"],
        "system",
        "expected trailing system-role message: {body}"
    );
    body["stream"] = serde_json::json!(false);
    let turn1 = post(body, api_key.clone()).await;
    let usage1 = &turn1["usage"];
    let turn1_prompt = usage1["input_tokens"].as_u64().unwrap_or_default()
        + usage1["cache_creation_input_tokens"]
            .as_u64()
            .unwrap_or_default()
        + usage1["cache_read_input_tokens"]
            .as_u64()
            .unwrap_or_default();

    request.transcript.push(TranscriptItem::AssistantMessage(
        roder_api::transcript::AssistantMessage {
            text: "OK.".to_string(),
            phase: None,
        },
    ));
    request
        .transcript
        .push(TranscriptItem::UserMessage(UserMessage::text(
            "Turn 2: say OK again.",
        )));
    request.instructions.developer_context =
        Some("Session context: connected account B-2, turn 2 (changed).".to_string());
    let mut body = roder_ext_anthropic::AnthropicEngine::map_request(&request);
    body["stream"] = serde_json::json!(false);
    let turn2 = post(body, api_key).await;
    let usage2 = &turn2["usage"];
    let cache_read = usage2["cache_read_input_tokens"]
        .as_u64()
        .unwrap_or_default();
    let uncached = usage2["input_tokens"].as_u64().unwrap_or_default();
    eprintln!(
        "live system-role context: turn1 prompt={turn1_prompt}, turn2 cache_read={cache_read} uncached={uncached}"
    );
    // The warm turn reads the stable prefix AND the conversation history from
    // cache despite the changed per-turn context; only the small tail (new
    // user message + volatile context) is uncached.
    assert!(
        cache_read as f64 >= turn1_prompt as f64 * 0.8,
        "expected turn 2 to read >=80% of turn 1's prompt from cache, got {cache_read}/{turn1_prompt}"
    );
    assert!(
        uncached < 1000,
        "expected a small uncached tail on the warm turn, got {uncached}"
    );
}

fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    format!(
        "{:x}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
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
