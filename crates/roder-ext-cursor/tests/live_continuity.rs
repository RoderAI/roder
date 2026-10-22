use futures::StreamExt;
use roder_api::inference::{
    AgentInferenceRequest, InferenceEngine, InferenceEvent, InferenceTurnContext,
    InstructionBundle, ModelSelection, OutputConfig, ReasoningConfig, RuntimeHints,
};
use roder_api::tools::ToolChoice;
use roder_api::transcript::{AssistantMessage, TranscriptItem, UserMessage};
use roder_ext_cursor::{CursorConfig, CursorInferenceEngine};
use serde_json::json;

fn request(transcript: Vec<TranscriptItem>) -> AgentInferenceRequest {
    AgentInferenceRequest {
        model: ModelSelection {
            provider: "cursor".into(),
            model: "composer-2.5".into(),
        },
        instructions: InstructionBundle::default(),
        transcript,
        tools: vec![],
        tool_choice: ToolChoice::Auto,
        reasoning: ReasoningConfig::default(),
        output: OutputConfig::default(),
        runtime: RuntimeHints::default(),
        metadata: json!({}),
    }
}
async fn turn(
    engine: &CursorInferenceEngine,
    thread: &str,
    transcript: Vec<TranscriptItem>,
) -> (String, Vec<TranscriptItem>) {
    let mut stream = engine
        .stream_turn(
            InferenceTurnContext {
                thread_id: thread,
                turn_id: "continuity-test",
                tool_executor: None,
            },
            request(transcript),
        )
        .await
        .unwrap();
    let mut text = String::new();
    let mut metadata = vec![];
    while let Some(event) = stream.next().await {
        match event.unwrap() {
            InferenceEvent::MessageDelta(delta) => text.push_str(&delta.text),
            InferenceEvent::ProviderMetadata(value) => {
                metadata.push(TranscriptItem::ProviderMetadata(value))
            }
            _ => {}
        }
    }
    (text, metadata)
}

#[tokio::test]
#[ignore = "requires RODER_CURSOR_LIVE=1 and Cursor credentials"]
async fn live_cursor_checkpoint_restores_prior_turn_without_tools() {
    assert_eq!(std::env::var("RODER_CURSOR_LIVE").as_deref(), Ok("1"));
    let engine = CursorInferenceEngine::new(CursorConfig::default());
    let thread = format!("cursor-state-{}", uuid::Uuid::new_v4());
    let secret = format!("violet-{}", uuid::Uuid::new_v4().simple());
    let first = TranscriptItem::UserMessage(UserMessage::text(format!(
        "Remember this verification phrase for our next turn: {secret}. Reply with exactly SAVED."
    )));
    let (text, metadata) = turn(&engine, &thread, vec![first.clone()]).await;
    assert!(text.contains("SAVED"), "save acknowledgement: {text}");
    assert!(metadata.iter().any(|item|matches!(item,TranscriptItem::ProviderMetadata(value) if value["cursorState"]["checkpoint"].as_str().is_some_and(|state|!state.is_empty()))));
    let mut transcript = vec![
        first,
        TranscriptItem::AssistantMessage(AssistantMessage { text, phase: None }),
    ];
    transcript.extend(metadata);
    transcript.push(TranscriptItem::UserMessage(UserMessage::text("What verification phrase did I ask you to remember? Reply with only that phrase. Do not use tools.")));
    let (recalled, _) = turn(&engine, &thread, transcript).await;
    assert_eq!(recalled.trim(), secret);
}

#[tokio::test]
#[ignore = "requires RODER_CURSOR_LIVE=1 and Cursor credentials"]
async fn live_cursor_imports_text_history_without_a_checkpoint() {
    assert_eq!(std::env::var("RODER_CURSOR_LIVE").as_deref(), Ok("1"));
    let engine = CursorInferenceEngine::new(CursorConfig::default());
    let secret = format!("cedar-{}", uuid::Uuid::new_v4().simple());
    let transcript = vec![
        TranscriptItem::UserMessage(UserMessage::text("What is the verification phrase?")),
        TranscriptItem::AssistantMessage(AssistantMessage {
            text: secret.clone(),
            phase: None,
        }),
        TranscriptItem::UserMessage(UserMessage::text(
            "What phrase did the previous assistant message contain? Reply only with that phrase, without tools.",
        )),
    ];
    let (text, _) = turn(
        &engine,
        &format!("cursor-import-{}", uuid::Uuid::new_v4()),
        transcript,
    )
    .await;
    assert_eq!(text.trim(), secret);
}
