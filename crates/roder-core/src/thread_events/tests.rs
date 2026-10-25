use super::*;
use roder_api::{extension::ExtensionRegistryBuilder, inference::*, transcript::TranscriptItem};
use roder_ext_jsonl_thread_store::store::JsonlThreadStoreFactory;
use tokio::sync::{Semaphore, mpsc};

struct Engine {
    requests: mpsc::UnboundedSender<(String, AgentInferenceRequest)>,
    gate: Option<Arc<Semaphore>>,
    call_tool: AtomicBool,
}
#[async_trait::async_trait]
impl InferenceEngine for Engine {
    fn id(&self) -> String {
        "mock".into()
    }
    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities::coding_agent_default()
    }
    async fn list_models(
        &self,
        _: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(vec![])
    }
    async fn stream_turn(
        &self,
        ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        self.requests.send((ctx.thread_id.into(), request)).unwrap();
        if let Some(gate) = &self.gate {
            gate.acquire().await.unwrap().forget();
        }
        let events = if self.call_tool.swap(false, Ordering::AcqRel) {
            vec![InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                id:"subscribe-call".into(), name:"subscribe_event".into(),
                arguments: serde_json::json!({"topic":"ci:167","prompt":"Check CI; cancel when finished","intervalSeconds":30,"maxDeliveries":2}).to_string(),
            }), InferenceEvent::Completed(CompletionMetadata { stop_reason:Some("tool_calls".into()),provider_response_id:None })]
        } else {
            vec![InferenceEvent::Completed(CompletionMetadata {
                stop_reason: Some("stop".into()),
                provider_response_id: None,
            })]
        };
        Ok(Box::pin(futures::stream::iter(events.into_iter().map(Ok))))
    }
}

fn harness(
    blocked: bool,
    call_tool: bool,
) -> (
    Arc<Runtime>,
    mpsc::UnboundedReceiver<(String, AgentInferenceRequest)>,
    Arc<Semaphore>,
    tempfile::TempDir,
) {
    let dir = tempfile::tempdir().unwrap();
    let (tx, rx) = mpsc::unbounded_channel();
    let gate = Arc::new(Semaphore::new(0));
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(Engine {
        requests: tx,
        gate: blocked.then(|| gate.clone()),
        call_tool: AtomicBool::new(call_tool),
    }));
    builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
        base_path: dir.path().join("threads"),
    }));
    let runtime = Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            crate::RuntimeConfig {
                team_data_dir: Some(dir.path().join("teams")),
                ..Default::default()
            },
        )
        .unwrap(),
    );
    (runtime, rx, gate, dir)
}
fn request(thread_id: &str) -> StartTurnRequest {
    StartTurnRequest {
        thread_id: thread_id.into(),
        message: "Watch this PR".into(),
        images: vec![],
        provider_override: Some("mock".into()),
        model_override: Some("mock".into()),
        reasoning_override: None,
        workspace: std::env::current_dir().unwrap().display().to_string(),
        instructions: InstructionBundle {
            system: Some("retained system".into()),
            developer: None,
            developer_context: None,
        },
        developer_context: Some("turn-only authorization".into()),
        task_ledger_required: false,
        service_tier_override: None,
    }
}
fn subscription() -> ThreadEventSubscribeRequest {
    ThreadEventSubscribeRequest {
        topic: "ci:167".into(),
        prompt: "Check CI; cancel when green".into(),
        delay_seconds: None,
        interval_seconds: Some(30),
        max_deliveries: 2,
        ttl_seconds: 3600,
    }
}
async fn received(
    rx: &mut mpsc::UnboundedReceiver<(String, AgentInferenceRequest)>,
) -> (String, AgentInferenceRequest) {
    tokio::time::timeout(Duration::from_secs(4), rx.recv())
        .await
        .unwrap()
        .unwrap()
}
async fn idle(runtime: &Runtime, thread_id: &str) {
    tokio::time::timeout(Duration::from_secs(4), async {
        while runtime
            .has_active_turn_for_thread(&thread_id.to_string())
            .await
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn timers_coalesce_while_busy_then_resume_same_thread_and_stop_at_limit() {
    let (runtime, mut rx, gate, _dir) = harness(true, false);
    runtime.start_turn(request("thread-1")).await.unwrap();
    received(&mut rx).await;
    let created = runtime
        .subscribe_thread_event("thread-1", subscription())
        .await
        .unwrap();
    assert!(created.next_tick_at.is_some());
    assert!(created.last_delivered_at.is_none());
    let now = Instant::now();
    runtime
        .tick_thread_events(now + Duration::from_secs(90))
        .await;
    runtime
        .tick_thread_events(now + Duration::from_secs(180))
        .await;
    assert!(rx.try_recv().is_err());
    assert!(runtime.list_thread_event_subscriptions("thread-1").await[0].pending);
    gate.add_permits(10);
    idle(&runtime, "thread-1").await;
    runtime
        .tick_thread_events(now + Duration::from_secs(181))
        .await;
    let (thread, next) = received(&mut rx).await;
    assert_eq!(thread, "thread-1");
    assert_eq!(next.model.model, "mock");
    let text = serde_json::to_string(&next).unwrap();
    assert!(text.contains("retained system"));
    assert!(!text.contains("turn-only authorization"));
    assert!(
        text.contains("Watch this PR"),
        "prior conversation survives follow-up"
    );
    assert!(text.contains("Scheduled automation tick"));
    idle(&runtime, "thread-1").await;
    let inspected = runtime.list_thread_event_subscriptions("thread-1").await;
    assert!(inspected[0].next_tick_at.is_some());
    assert!(inspected[0].last_delivered_at.is_some());
    let json = serde_json::to_value(&inspected[0]).unwrap();
    assert!(json["nextTickAt"].as_str().unwrap().ends_with('Z'));
    assert!(json["lastDeliveredAt"].as_str().unwrap().ends_with('Z'));
    runtime
        .tick_thread_events(now + Duration::from_secs(220))
        .await;
    received(&mut rx).await;
    idle(&runtime, "thread-1").await;
    assert!(
        runtime
            .list_thread_event_subscriptions("thread-1")
            .await
            .is_empty()
    );
    runtime
        .tick_thread_events(now + Duration::from_secs(260))
        .await;
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn topics_route_only_to_subscribers_and_cancellation_is_thread_scoped() {
    let (runtime, mut rx, _, _dir) = harness(false, false);
    for id in ["first", "second"] {
        runtime.start_turn(request(id)).await.unwrap();
        received(&mut rx).await;
        idle(&runtime, id).await;
    }
    let mut args = subscription();
    args.interval_seconds = None;
    let sub = runtime.subscribe_thread_event("first", args).await.unwrap();
    assert_eq!(
        runtime
            .publish_thread_event("other", "ignored")
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        runtime.publish_thread_event("ci:167", "old").await.unwrap(),
        1
    );
    runtime
        .publish_thread_event("ci:167", "latest CI result")
        .await
        .unwrap();
    runtime.tick_thread_events(Instant::now()).await;
    let (thread, input) = received(&mut rx).await;
    assert_eq!(thread, "first");
    let last = input
        .transcript
        .iter()
        .rev()
        .find_map(|i| match i {
            TranscriptItem::UserMessage(m) => Some(&m.text),
            _ => None,
        })
        .unwrap();
    assert!(last.contains("latest CI result"));
    assert!(!last.contains("old"));
    assert!(
        !runtime
            .cancel_thread_event_subscription("second", &sub.id)
            .await
    );
    assert!(
        runtime
            .cancel_thread_event_subscription("first", &sub.id)
            .await
    );
    idle(&runtime, "first").await;
    assert_eq!(
        runtime
            .publish_thread_event("ci:167", "late")
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn real_clock_delivers_one_shot_without_a_client_tick() {
    let (runtime, mut rx, _, _dir) = harness(false, false);
    runtime.start_turn(request("once")).await.unwrap();
    received(&mut rx).await;
    idle(&runtime, "once").await;
    let mut args = subscription();
    args.interval_seconds = None;
    args.delay_seconds = Some(1);
    runtime.subscribe_thread_event("once", args).await.unwrap();
    let (thread_id, delivered) = received(&mut rx).await;
    assert_eq!(thread_id, "once");
    let text = serde_json::to_string(&delivered.transcript).unwrap();
    assert!(text.contains("exhausted automatically; no cancellation call is needed"));
    assert!(!text.contains("Cancel this subscription when the work is done"));
    idle(&runtime, "once").await;
    assert!(
        runtime
            .list_thread_event_subscriptions("once")
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn model_can_create_subscription_through_normal_tool_execution() {
    let (runtime, mut rx, _, _dir) = harness(false, true);
    runtime.start_turn(request("tool-caller")).await.unwrap();
    let (_, initial) = received(&mut rx).await;
    assert!(initial.tools.iter().any(|t| t.name == "subscribe_event"));
    let (_, next) = received(&mut rx).await;
    let serialized = serde_json::to_string(&next.transcript).unwrap();
    assert!(serialized.contains("subscribe-call"), "{serialized}");
    assert!(serialized.contains("remainingDeliveries"));
    idle(&runtime, "tool-caller").await;
    assert_eq!(
        runtime
            .list_thread_event_subscriptions("tool-caller")
            .await
            .len(),
        1
    );
}

#[tokio::test]
async fn interrupt_discards_subscriptions_and_pending_events() {
    let (runtime, mut rx, _, _dir) = harness(true, false);
    let turn = runtime.start_turn(request("stop")).await.unwrap();
    received(&mut rx).await;
    runtime
        .subscribe_thread_event("stop", subscription())
        .await
        .unwrap();
    runtime
        .publish_thread_event("ci:167", "pending")
        .await
        .unwrap();
    runtime.interrupt_turn("stop".into(), turn).await.unwrap();
    assert!(
        runtime
            .list_thread_event_subscriptions("stop")
            .await
            .is_empty()
    );
    runtime
        .tick_thread_events(Instant::now() + Duration::from_secs(90))
        .await;
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn rejects_unknown_threads_invalid_timers_and_expired_subscriptions() {
    let (runtime, mut rx, _, _dir) = harness(false, false);
    assert!(
        runtime
            .subscribe_thread_event("unknown", subscription())
            .await
            .is_err()
    );
    runtime.start_turn(request("valid")).await.unwrap();
    received(&mut rx).await;
    idle(&runtime, "valid").await;
    let mut args = subscription();
    args.interval_seconds = Some(0);
    assert!(runtime.subscribe_thread_event("valid", args).await.is_err());
    let mut args = subscription();
    args.ttl_seconds = 1;
    runtime.subscribe_thread_event("valid", args).await.unwrap();
    runtime
        .tick_thread_events(Instant::now() + Duration::from_secs(2))
        .await;
    assert!(
        runtime
            .list_thread_event_subscriptions("valid")
            .await
            .is_empty()
    );
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn quiescing_runtime_does_not_admit_pending_ticks_or_accept_publications() {
    let (runtime, mut rx, _, _dir) = harness(false, false);
    runtime.start_turn(request("drain")).await.unwrap();
    received(&mut rx).await;
    idle(&runtime, "drain").await;
    runtime
        .subscribe_thread_event("drain", subscription())
        .await
        .unwrap();
    runtime
        .publish_thread_event("ci:167", "CI pending")
        .await
        .unwrap();
    runtime.drain_active_turns(Duration::from_secs(1)).await;
    runtime
        .tick_thread_events(Instant::now() + Duration::from_secs(31))
        .await;
    assert!(rx.try_recv().is_err());
    assert!(
        runtime
            .publish_thread_event("ci:167", "late")
            .await
            .is_err()
    );
    assert!(
        runtime
            .subscribe_thread_event("drain", subscription())
            .await
            .is_err()
    );
    runtime.resume_accepting_turns();
    runtime.tick_thread_events(Instant::now()).await;
    assert_eq!(received(&mut rx).await.0, "drain");
    idle(&runtime, "drain").await;
}

#[tokio::test]
async fn paused_goals_defer_timer_delivery_without_consuming_it() {
    use roder_api::goals::{ThreadGoalPatch, ThreadGoalStatus};
    let (runtime, mut rx, _, _dir) = harness(false, false);
    runtime.start_turn(request("paused")).await.unwrap();
    received(&mut rx).await;
    idle(&runtime, "paused").await;
    runtime
        .subscribe_thread_event("paused", subscription())
        .await
        .unwrap();
    runtime
        .thread_goal_set(
            &"paused".into(),
            ThreadGoalPatch {
                objective: Some("Watch CI".into()),
                status: Some(ThreadGoalStatus::Paused),
                token_budget: None,
            },
        )
        .await
        .unwrap();
    runtime
        .tick_thread_events(Instant::now() + Duration::from_secs(31))
        .await;
    assert!(rx.try_recv().is_err());
    let subscriptions = runtime.list_thread_event_subscriptions("paused").await;
    assert_eq!(subscriptions[0].remaining_deliveries, 2);
    assert!(subscriptions[0].pending);
}

#[test]
fn scheduling_schema_distinguishes_one_shot_from_recurring_timers() {
    use roder_api::tools::ToolRegistry;
    let mut registry = ToolRegistry::default();
    super::contribute_tools(&mut registry).unwrap();
    let spec = registry.get("subscribe_event").unwrap().spec();
    assert_eq!(
        spec.parameters["properties"]["intervalSeconds"]["type"],
        serde_json::json!(["integer", "null"])
    );
    let args:ThreadEventSubscribeRequest=serde_json::from_value(serde_json::json!({"topic":"once","prompt":"remind","delaySeconds":8,"intervalSeconds":null})).unwrap();
    assert_eq!(args.interval_seconds, None);
}
