// Screenshot output must retain observations, call identity, and resolution.
use super::*;

fn view_image_transcript() -> Vec<TranscriptItem> {
    vec![
        TranscriptItem::ToolCall(ToolCallRecord {
            id: "call_img".to_string(),
            name: "view_image".to_string(),
            arguments: "{\"path\":\"board.png\"}".to_string(),
        }),
        TranscriptItem::ToolResult(ToolResultRecord {
            id: "call_img".to_string(),
            name: Some("view_image".to_string()),
            result: "Viewing image board.png".to_string(),
            display_payload: Some(json!({
                "__view_image": {
                    "image_url": "data:image/png;base64,YWJj",
                    "detail": "original"
                }
            })),
            is_error: false,
        }),
    ]
}

#[test]
fn forwards_view_image_output_as_input_image() {
    let mut request = request();
    request.transcript = view_image_transcript();

    let input = input_items(&request);
    let output = input
        .iter()
        .find(|item| item["type"] == "function_call_output")
        .expect("tool output present");
    let content = output["output"].as_array().expect("image array output");
    assert_eq!(output["call_id"], "call_img");
    assert_eq!(content[0]["type"], "input_text");
    assert_eq!(content[0]["text"], "Viewing image board.png");
    assert_eq!(content[1]["type"], "input_image");
    assert_eq!(content[1]["image_url"], "data:image/png;base64,YWJj");
    assert_eq!(content[1]["detail"], "original");
}

#[test]
fn view_image_output_falls_back_to_string_without_image_support() {
    let mut request = request();
    request.transcript = view_image_transcript();

    let (_, tool_name_map) = responses_tools(&request, ResponsesProviderProfile::OpenAi);
    let input = response_input_items(
        &request,
        &tool_name_map,
        ResponsesProviderProfile::OpenAi,
        false,
    );
    let output = input
        .iter()
        .find(|item| item["type"] == "function_call_output")
        .expect("tool output present");
    assert_eq!(output["output"], "Viewing image board.png");
}

fn forwards_tool_image(provider: &str, model: &str) -> (bool, OpenAiResponsesEngine) {
    let engine = OpenAiResponsesEngine::new_with_config(
        None,
        provider,
        "http://localhost/v1",
        Vec::new(),
    );
    let mut request = request();
    request.model.provider = provider.to_string();
    request.model.model = model.to_string();
    request.transcript = view_image_transcript();
    let (body, _) = OpenAiResponsesEngine::map_request_with_options(
        &request,
        RequestMappingOptions {
            profile: engine.profile,
            thread_id: None,
        },
    );
    let forwarded = body["input"]
        .as_array()
        .expect("input items")
        .iter()
        .filter(|item| item["type"] == "function_call_output")
        .filter_map(|item| item["output"].as_array())
        .flatten()
        .any(|block| block["type"] == "input_image");
    (forwarded, engine)
}

#[test]
fn engine_reports_tool_result_images_exactly_when_the_replay_emits_input_image() {
    for (provider, model, expected) in [
        (PROVIDER_OPENAI, "gpt-5.5", true),
        (PROVIDER_CODEX, "gpt-5.5", true),
        // Kept as is: assumed to take images whatever the model.
        (PROVIDER_OPENROUTER, "vendor/any-model", true),
        (PROVIDER_FIREWORKS, "accounts/any/models/any", true),
        ("custom-responses-provider", "any-model", true),
        // xAI follows the catalog; a model the catalog does not know is assumed to take images.
        (PROVIDER_SUPERGROK, "grok-4.6", true),
        (PROVIDER_SUPERGROK, "grok-composer-2.5-fast", false),
        (PROVIDER_XAI, "model-the-catalog-lacks", true),
    ] {
        let (forwarded, engine) = forwards_tool_image(provider, model);
        assert_eq!(forwarded, expected, "{provider}/{model} replay");
        assert_eq!(
            engine.tool_result_image_input(model),
            expected,
            "{provider}/{model} engine"
        );
        assert_eq!(
            forwards_tool_result_images(provider, model),
            expected,
            "{provider}/{model} shared helper"
        );
    }
}
