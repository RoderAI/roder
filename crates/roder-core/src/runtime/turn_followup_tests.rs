use super::*;
use roder_api::extension::ExtensionRegistryBuilder;
use std::time::Duration;

fn followup_options(thread_id: &ThreadId, workspace: &std::path::Path) -> StartTurnRequest {
    StartTurnRequest {
        thread_id: thread_id.clone(),
        message: String::new(),
        images: Vec::new(),
        provider_override: None,
        model_override: None,
        reasoning_override: None,
        workspace: workspace.display().to_string(),
        instructions: InstructionBundle::default(),
        developer_context: None,
        task_ledger_required: false,
        service_tier_override: None,
    }
}

async fn finished_turn_with_steers(runtime: &Arc<Runtime>, thread_id: &ThreadId) -> TurnId {
    let turn_id = "finished-turn".to_string();
    let (abort, _registration) = AbortHandle::new_pair();
    runtime.active_turns.write().await.insert(
        turn_id.clone(),
        ActiveTurnHandle {
            thread_id: thread_id.clone(),
            abort,
            steers: Arc::new(Mutex::new(Vec::new())),
            steer_changed: tokio::sync::watch::channel(0).0,
            drain: Arc::new(TurnDrainHandle {
                thread_id: thread_id.clone(),
                interrupt_requested: AtomicBool::new(false),
                interrupt_reason: Mutex::new(None),
                completed: AtomicBool::new(false),
                completed_notify: Notify::new(),
            }),
        },
    );
    // Arrives after the turn's last inference boundary, as a host draining its
    // queue on TurnCompleted does while teardown is still in flight.
    for text in ["first late input", "second late input"] {
        runtime
            .steer_turn(thread_id.clone(), turn_id.clone(), text.into(), vec![])
            .await
            .unwrap();
    }
    turn_id
}

fn fake_runtime() -> Arc<Runtime> {
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(crate::fake_provider::FakeInferenceEngine));
    Arc::new(Runtime::new(builder.build().unwrap(), Default::default()).unwrap())
}

#[tokio::test]
async fn late_steers_start_a_followup_turn_instead_of_vanishing() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = fake_runtime();
    let thread_id = runtime.create_thread(None).await.unwrap().thread_id;
    let finished = finished_turn_with_steers(&runtime, &thread_id).await;
    let mut events = runtime.subscribe_events();

    let steers = runtime.take_unconsumed_turn_steers(&finished).await;
    assert!(runtime.active_turn_for_thread(&thread_id).await.is_none());
    assert!(
        runtime
            .steer_turn(
                thread_id.clone(),
                finished.clone(),
                "too late".into(),
                vec![]
            )
            .await
            .is_err(),
        "a removed turn must reject steers rather than silently buffer them"
    );
    let replayed = runtime
        .replay_unconsumed_turn_steers(followup_options(&thread_id, directory.path()), steers)
        .await
        .unwrap();
    assert!(replayed);

    let message = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let RoderEvent::TranscriptItemAppended(appended) = events.recv().await.unwrap().event
                && let Some(TranscriptItem::UserMessage(message)) = appended.item
            {
                break message.text;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(message, "first late input\n\nsecond late input");
}

#[tokio::test]
async fn late_steers_join_a_turn_that_already_started() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = fake_runtime();
    let thread_id = runtime.create_thread(None).await.unwrap().thread_id;
    let finished = finished_turn_with_steers(&runtime, &thread_id).await;
    let steers = runtime.take_unconsumed_turn_steers(&finished).await;
    let (abort, _registration) = AbortHandle::new_pair();
    let newer_steers = Arc::new(Mutex::new(Vec::new()));
    runtime.active_turns.write().await.insert(
        "newer-turn".into(),
        ActiveTurnHandle {
            thread_id: thread_id.clone(),
            abort,
            steers: newer_steers.clone(),
            steer_changed: tokio::sync::watch::channel(0).0,
            drain: Arc::new(TurnDrainHandle {
                thread_id: thread_id.clone(),
                interrupt_requested: AtomicBool::new(false),
                interrupt_reason: Mutex::new(None),
                completed: AtomicBool::new(false),
                completed_notify: Notify::new(),
            }),
        },
    );

    assert!(
        runtime
            .replay_unconsumed_turn_steers(followup_options(&thread_id, directory.path()), steers)
            .await
            .unwrap()
    );
    let joined = newer_steers.lock().await;
    assert!(matches!(
        joined.as_slice(),
        [steer] if steer.message.text == "first late input\n\nsecond late input"
    ));
}
