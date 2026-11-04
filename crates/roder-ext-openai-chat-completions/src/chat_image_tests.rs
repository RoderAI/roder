use roder_api::inference::{AgentInferenceRequest, InferenceEngine, ModelSelection};
use roder_api::tools::ToolChoice;
use roder_api::transcript::{
    InputImage, ToolCallRecord, ToolResultRecord, TranscriptItem, UserMessage,
    VIEW_IMAGE_DISPLAY_KEY,
};
use serde_json::{Value, json};

use super::{ChatToolNameMap, chat_messages};
use crate::OpenAiChatCompletionsEngine;

fn request(transcript: Vec<TranscriptItem>) -> AgentInferenceRequest {
    AgentInferenceRequest {
        model: ModelSelection {
            provider: "hopper".into(),
            model: "deepseek-v4.1-flash-1017".into(),
        },
        instructions: Default::default(),
        transcript,
        tools: Vec::new(),
        tool_choice: ToolChoice::None,
        reasoning: Default::default(),
        output: Default::default(),
        runtime: Default::default(),
        metadata: json!({}),
    }
}

fn messages(transcript: Vec<TranscriptItem>) -> Vec<Value> {
    chat_messages(&request(transcript), &ChatToolNameMap::default())
}

fn user(text: &str, urls: &[&str]) -> TranscriptItem {
    TranscriptItem::UserMessage(UserMessage::with_images(
        text,
        urls.iter()
            .map(|url| InputImage {
                image_url: (*url).into(),
            })
            .collect(),
    ))
}

fn call(id: &str) -> TranscriptItem {
    TranscriptItem::ToolCall(ToolCallRecord {
        id: id.into(),
        name: "view_image".into(),
        arguments: "{}".into(),
    })
}

fn result(id: &str, url: &str) -> TranscriptItem {
    TranscriptItem::ToolResult(ToolResultRecord {
        id: id.into(),
        name: Some("view_image".into()),
        result: format!("result {id}"),
        display_payload: Some(json!({ VIEW_IMAGE_DISPLAY_KEY: { "image_url": url } })),
        is_error: false,
    })
}

#[test]
fn user_images_preserve_text_remote_urls_data_urls_and_order() {
    let urls = [
        "https://example.com/photo.png",
        "data:image/jpeg;base64,YWJj",
    ];
    let mapped = messages(vec![user("Compare these", &urls)]);
    assert_eq!(
        mapped,
        vec![json!({
            "role": "user",
            "content": [
                { "type": "text", "text": "Compare these" },
                { "type": "image_url", "image_url": { "url": urls[0] } },
                { "type": "image_url", "image_url": { "url": urls[1] } }
            ]
        })]
    );
}

#[test]
fn image_only_messages_omit_empty_text_parts() {
    let mapped = messages(vec![user("", &["https://example.com/image.png"])]);
    assert_eq!(
        mapped[0]["content"],
        json!([{ "type": "image_url", "image_url": { "url": "https://example.com/image.png" } }])
    );
}

#[test]
fn text_only_messages_keep_string_content_in_both_transports() {
    for text in ["hello", ""] {
        let req = request(vec![user(text, &[])]);
        assert_eq!(
            chat_messages(&req, &ChatToolNameMap::default())[0]["content"],
            json!(text)
        );
        let engine = OpenAiChatCompletionsEngine::new("test".into());
        assert_eq!(engine.map_request(&req).messages[0].content, json!(text));
    }
}

#[test]
fn non_streaming_engine_serializes_multimodal_user_content() {
    let engine = OpenAiChatCompletionsEngine::new("test".into());
    assert!(engine.capabilities().image_input);
    let req = request(vec![user("Describe", &["data:image/png;base64,YWJj"])]);
    let mapped = serde_json::to_value(engine.map_request(&req)).unwrap();
    assert_eq!(
        mapped["messages"],
        json!(chat_messages(&req, &ChatToolNameMap::default()))
    );
}

#[test]
fn tool_images_follow_all_parallel_results_even_when_recorded_out_of_order() {
    let mapped = messages(vec![
        call("a"),
        call("b"),
        result("b", "data:image/jpeg;base64,YWJj"),
        result("a", "data:image/png;base64,YWJj"),
    ]);
    let roles: Vec<_> = mapped.iter().map(|m| m["role"].as_str().unwrap()).collect();
    assert_eq!(roles, ["assistant", "tool", "tool", "user", "user"]);
    assert_eq!(mapped[0]["tool_calls"].as_array().unwrap().len(), 2);
    assert_eq!(mapped[1]["tool_call_id"], "a");
    assert_eq!(mapped[1]["content"], "result a");
    assert_eq!(mapped[2]["tool_call_id"], "b");
    assert_eq!(mapped[2]["content"], "result b");
    assert_eq!(mapped[3]["content"][0]["text"], "Image from tool call a:");
    assert_eq!(
        mapped[3]["content"][1]["image_url"]["url"],
        "data:image/png;base64,YWJj"
    );
    assert_eq!(
        mapped[4]["content"][1]["image_url"]["url"],
        "data:image/jpeg;base64,YWJj"
    );
}

#[test]
fn invalid_or_external_tool_images_are_not_forwarded() {
    for url in [
        "https://example.com/image.png",
        "data:image/svg+xml;base64,YWJj",
        "data:image/png;base64,invalid!",
        "data:image/png;base64,",
    ] {
        let mapped = messages(vec![call("a"), result("a", url)]);
        assert_eq!(mapped.len(), 2, "unexpected image for {url}");
        assert_eq!(mapped[1]["content"], "result a");
    }
    assert!(messages(vec![result("orphan", "data:image/png;base64,YWJj")]).is_empty());
}

#[test]
fn missing_parallel_result_stays_adjacent_before_image() {
    let mapped = messages(vec![
        call("a"),
        call("missing"),
        result("a", "data:image/png;base64,YWJj"),
    ]);
    assert_eq!(mapped.len(), 4);
    assert_eq!(mapped[1]["role"], "tool");
    assert_eq!(mapped[2]["role"], "tool");
    assert_eq!(mapped[2]["tool_call_id"], "missing");
    assert_eq!(mapped[3]["role"], "user");
}
