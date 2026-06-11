#[tokio::test]
#[ignore = "requires RODER_OPENAI_LIVE=1 and OPENAI_API_KEY"]
async fn live_openai_tools_smoke_is_explicitly_opt_in() {
    if std::env::var("RODER_OPENAI_LIVE").ok().as_deref() != Some("1") {
        eprintln!("set RODER_OPENAI_LIVE=1 to run live OpenAI tool smoke tests");
        return;
    }

    let has_key = std::env::var("OPENAI_API_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .is_some();

    assert!(
        has_key,
        "live OpenAI tool smoke tests require OPENAI_API_KEY"
    );
}

/**
 * Opt-in live validation that the provider-native tool-search request body
 * produced by the real request mapper is accepted by the OpenAI Responses
 * API (`tool_search` entry plus `defer_loading` function tools).
 */
#[tokio::test]
#[ignore = "requires RODER_OPENAI_TOOL_SEARCH_LIVE=1 and OPENAI_API_KEY"]
async fn live_openai_tool_search_smoke_is_explicitly_opt_in() {
    if std::env::var("RODER_OPENAI_TOOL_SEARCH_LIVE")
        .ok()
        .as_deref()
        != Some("1")
    {
        eprintln!("set RODER_OPENAI_TOOL_SEARCH_LIVE=1 to run live OpenAI tool-search tests");
        return;
    }

    let api_key = std::env::var("OPENAI_API_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .expect("live OpenAI tool-search smoke tests require OPENAI_API_KEY");
    let model = std::env::var("RODER_OPENAI_TOOL_SEARCH_MODEL")
        .unwrap_or_else(|_| "gpt-5.5".to_string());

    let mut request = tool_search_request(&model);
    request.runtime.tool_search =
        roder_api::inference::ToolSearchConfig::provider_native();
    let mut body =
        roder_ext_openai_responses::OpenAiResponsesEngine::map_request(&request);
    assert_eq!(
        body["tools"].as_array().map(Vec::len),
        Some(3),
        "expected two deferred tools plus the tool_search entry: {body}"
    );
    body["stream"] = serde_json::json!(false);

    let response = reqwest::Client::new()
        .post("https://api.openai.com/v1/responses")
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
        .expect("send live OpenAI tool-search request");
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    assert!(
        status.is_success(),
        "live OpenAI tool-search request rejected ({status}): {text}"
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
            provider: "openai".to_string(),
            model: model.to_string(),
        },
        instructions: InstructionBundle {
            system: Some("You are a coding agent; search for the right tool.".to_string()),
            developer: None,
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
