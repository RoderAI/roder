use futures::StreamExt;
use std::path::PathBuf;

use roder_api::catalog::PROVIDER_CURSOR;
use roder_api::inference::{
    AgentInferenceRequest, InferenceEngine, InferenceEvent, InferenceTurnContext,
    InstructionBundle, ModelSelection, OutputConfig, ReasoningConfig, RuntimeHints,
};
use roder_api::tools::ToolChoice;
use roder_api::transcript::{TranscriptItem, UserMessage};
use roder_ext_cursor::{CursorConfig, CursorInferenceEngine};
use serde_json::json;

#[tokio::test]
#[ignore = "requires RODER_CURSOR_LIVE=1 and a valid CURSOR_API_KEY or RODER_CURSOR_API_KEY"]
async fn live_cursor_composer_25_proof_token() {
    if std::env::var("RODER_CURSOR_LIVE").ok().as_deref() != Some("1") {
        eprintln!("set RODER_CURSOR_LIVE=1 to run live Cursor Composer test");
        return;
    }

    let proof = "RODER_CURSOR_NATIVE_PROVIDER_LIVE_PROOF";
    let engine = CursorInferenceEngine::new(CursorConfig {
        workspace: Some(workspace_root()),
        ..CursorConfig::default()
    });
    let request = AgentInferenceRequest {
        model: ModelSelection {
            provider: PROVIDER_CURSOR.to_string(),
            model: "composer-2.5".to_string(),
        },
        instructions: InstructionBundle::default(),
        transcript: vec![TranscriptItem::UserMessage(UserMessage::text(format!(
            "Reply with exactly this token and nothing else: {proof}"
        )))],
        tools: Vec::new(),
        tool_choice: ToolChoice::Auto,
        reasoning: ReasoningConfig::default(),
        output: OutputConfig::default(),
        runtime: RuntimeHints::default(),
        metadata: json!({}),
    };

    let mut stream = engine
        .stream_turn(
            InferenceTurnContext {
                thread_id: "live-cursor-thread",
                turn_id: "live-cursor-turn",
            },
            request,
        )
        .await
        .expect("live Cursor request should start");
    let mut text = String::new();
    while let Some(event) = stream.next().await {
        if let InferenceEvent::MessageDelta(delta) = event.expect("live Cursor event") {
            text.push_str(&delta.text);
        }
    }

    assert_eq!(text.trim(), proof);
}

#[tokio::test]
#[ignore = "requires RODER_CURSOR_LIVE=1 and a valid CURSOR_API_KEY or RODER_CURSOR_API_KEY"]
async fn live_cursor_composer_25_short_story() {
    if std::env::var("RODER_CURSOR_LIVE").ok().as_deref() != Some("1") {
        eprintln!("set RODER_CURSOR_LIVE=1 to run live Cursor Composer test");
        return;
    }

    let engine = CursorInferenceEngine::new(CursorConfig {
        workspace: Some(workspace_root()),
        ..CursorConfig::default()
    });
    let request = AgentInferenceRequest {
        model: ModelSelection {
            provider: PROVIDER_CURSOR.to_string(),
            model: "composer-2.5".to_string(),
        },
        instructions: InstructionBundle::default(),
        transcript: vec![TranscriptItem::UserMessage(UserMessage::text(
            "Write a short story in five sentences about a lighthouse keeper and a lost signal. Return only the story.",
        ))],
        tools: Vec::new(),
        tool_choice: ToolChoice::Auto,
        reasoning: ReasoningConfig::default(),
        output: OutputConfig::default(),
        runtime: RuntimeHints::default(),
        metadata: json!({}),
    };

    let mut stream = engine
        .stream_turn(
            InferenceTurnContext {
                thread_id: "live-cursor-thread",
                turn_id: "live-cursor-story-turn",
            },
            request,
        )
        .await
        .expect("live Cursor request should start");
    let mut text = String::new();
    while let Some(event) = stream.next().await {
        if let InferenceEvent::MessageDelta(delta) = event.expect("live Cursor event") {
            text.push_str(&delta.text);
        }
    }

    let sentence_count = text
        .chars()
        .filter(|ch| matches!(ch, '.' | '!' | '?'))
        .count();
    assert!(
        sentence_count >= 3,
        "expected story-like response, got: {text}"
    );
    let lowercase = text.to_ascii_lowercase();
    assert!(
        lowercase.contains("signal") || lowercase.contains("radio"),
        "expected story to preserve the lost-signal prompt, got: {text}"
    );
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crate should live under crates/roder-ext-cursor")
        .to_path_buf()
}
