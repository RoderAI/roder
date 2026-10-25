use super::*;
use roder_api::extension::ExtensionRegistryBuilder;
use roder_core::{RuntimeConfig, fake_provider::FakeInferenceEngine};

#[tokio::test]
async fn backend_interruption_preserves_usage_in_history_and_terminal_event() {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FakeInferenceEngine));
    let runtime =
        Arc::new(Runtime::new(builder.build().unwrap(), RuntimeConfig::default()).unwrap());
    let mut events = runtime.subscribe_events();
    let thread: Thread = serde_json::from_value(json!({
        "id": "thread",
        "preview": "",
        "modelProvider": "backend",
        "model": "model",
        "createdAt": 0,
        "updatedAt": 0,
        "status": {"type": "idle", "activeTurnId": null, "activeFlags": []},
        "cwd": "/tmp"
    }))
    .unwrap();
    let thread = Arc::new(RwLock::new(thread));
    let routes = Arc::new(RwLock::new(HashMap::new()));
    let mut active_turn = String::new();
    let mut output = HashMap::new();
    let mut accumulated = None;
    let usage = TokenUsage {
        prompt_tokens: 100,
        completion_tokens: 20,
        total_tokens: 120,
        cached_prompt_tokens: 30,
        cache_creation_prompt_tokens: 10,
        service_tier: Some("priority".into()),
        ..TokenUsage::default()
    };
    for event in [
        BackendEvent::TurnStarted {
            turn_id: "interrupted".into(),
        },
        BackendEvent::Usage(usage.clone()),
        BackendEvent::TurnFinished {
            turn_id: "interrupted".into(),
            status: "interrupted".into(),
        },
    ] {
        emit_event(
            event,
            "thread",
            &mut active_turn,
            &mut output,
            &mut accumulated,
            &thread,
            &runtime,
            &routes,
        )
        .await;
    }

    let mut terminal = None;
    while let Ok(event) = events.try_recv() {
        if matches!(event.event, RoderEvent::TurnInterrupted(_)) {
            terminal = Some(event.event);
        }
    }
    let terminal = serde_json::to_value(terminal.expect("interrupted terminal event")).unwrap();
    let expected = serde_json::to_value(&usage).unwrap();
    assert_eq!(terminal["TurnInterrupted"]["usage"], expected);
    assert_eq!(accumulated, None, "terminal usage must be consumed");
    assert!(active_turn.is_empty());
    let thread = thread.read().await;
    assert_eq!(thread.status.kind, "idle");
    let turn = &thread.turns.as_ref().unwrap()[0];
    assert_eq!(turn.status, "interrupted");
    assert_eq!(turn.usage, Some(usage));
}
