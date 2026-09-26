use futures::stream;
use roder_api::{
    extension::ExtensionRegistryBuilder,
    inference::*,
    lifecycle::TurnCleanupOwnership,
    provider_error::{ProviderFailure, ProviderFailureKind},
};
use roder_core::{Runtime, RuntimeConfig, StartTurnRequest, default_instructions};
use serde_json::json;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

#[derive(Clone, Copy)]
enum Mode {
    Terminal(ProviderFailureKind),
    SetupCleanup,
    SharedBudget,
    ResetBudget,
    Replay(bool),
}
struct Engine {
    mode: Mode,
    requests: Mutex<Vec<AgentInferenceRequest>>,
    cleaned: Arc<AtomicBool>,
}
struct Cleanup(Arc<AtomicBool>);
#[async_trait::async_trait]
impl ProviderTurnCleanup for Cleanup {
    fn ownership(&self) -> TurnCleanupOwnership {
        TurnCleanupOwnership::ProviderCleanupPending
    }
    async fn wait_for_cleanup(&self) -> anyhow::Result<()> {
        tokio::time::sleep(Duration::from_millis(10)).await;
        self.0.store(true, Ordering::SeqCst);
        Ok(())
    }
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
        let n = {
            let mut requests = self.requests.lock().unwrap();
            requests.push(request);
            requests.len()
        };
        let failed = || {
            Err(
                ProviderFailure::new(ProviderFailureKind::StreamInterrupted, "fixture disconnect")
                    .into(),
            )
        };
        let complete = || {
            Ok(InferenceEvent::Completed(CompletionMetadata {
                stop_reason: Some("completed".into()),
                provider_response_id: None,
            }))
        };
        let events = match self.mode {
            Mode::Replay(changed) if n <= 2 => vec![
                Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: "once".into(),
                    name: "echo".into(),
                    arguments: if changed && n == 2 {
                        "{\"text\":\"changed\"}"
                    } else {
                        "{\"text\":\"ok\"}"
                    }
                    .into(),
                })),
                complete(),
            ],
            Mode::Replay(_) => vec![complete()],
            Mode::Terminal(kind) => {
                vec![Err(ProviderFailure::new(kind, "fixture terminal").into())]
            }
            Mode::SetupCleanup if n == 1 => {
                ctx.tool_executor
                    .unwrap()
                    .register_provider_cleanup(Arc::new(Cleanup(self.cleaned.clone())));
                return Err(ProviderFailure::new(
                    ProviderFailureKind::StreamInterrupted,
                    "fixture disconnect",
                )
                .into());
            }
            Mode::SetupCleanup => {
                assert!(
                    self.cleaned.load(Ordering::SeqCst),
                    "new sampling began before owned cleanup"
                );
                vec![complete()]
            }
            Mode::SharedBudget if n == 1 => vec![
                Ok(InferenceEvent::ProviderMetadata(
                    json!({"kind":"reliability_retry_attempt","attempt":1}),
                )),
                failed(),
            ],
            Mode::SharedBudget => vec![failed()],
            Mode::ResetBudget if n == 1 || n == 3 => vec![failed()],
            Mode::ResetBudget if n == 2 => vec![
                Ok(InferenceEvent::ToolCallCompleted(ToolCallCompleted {
                    id: "echo1".into(),
                    name: "echo".into(),
                    arguments: "{\"text\":\"ok\"}".into(),
                })),
                complete(),
            ],
            Mode::ResetBudget => vec![complete()],
        };
        Ok(Box::pin(stream::iter(events)))
    }
}

async fn exercise(mode: Mode) -> (Arc<Engine>, usize, usize, String) {
    exercise_profile(mode, RuntimeProfile::Interactive).await
}
async fn exercise_profile(
    mode: Mode,
    profile: RuntimeProfile,
) -> (Arc<Engine>, usize, usize, String) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".roder")).unwrap();
    std::fs::write(dir.path().join(".roder/hooks.json"),json!({"hooks":{"Stop":[{"hooks":[{"type":"command","command":"printf 'stop\\n' >> stop.txt"}]}]}}).to_string()).unwrap();
    let engine = Arc::new(Engine {
        mode,
        requests: Mutex::new(vec![]),
        cleaned: Arc::new(AtomicBool::new(false)),
    });
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(engine.clone());
    builder.tool_contributor(Arc::new(roder_tools::EchoToolContributor));
    let mut cfg = RuntimeConfig {
        workspace: Some(dir.path().to_string_lossy().into()),
        runtime_profile: profile,
        tool_allowlist: vec!["echo".into()],
        ..RuntimeConfig::default()
    };
    cfg.reliability.provider_retry_initial_backoff_ms = 0;
    let runtime = Arc::new(Runtime::new(builder.build().unwrap(), cfg).unwrap());
    let mut events = runtime.subscribe_events();
    runtime
        .start_turn(StartTurnRequest {
            thread_id: "recovery-fixture".into(),
            message: "go".into(),
            images: vec![],
            provider_override: None,
            model_override: None,
            reasoning_override: None,
            workspace: dir.path().to_string_lossy().into(),
            instructions: default_instructions(),
            developer_context: None,
            task_ledger_required: false,
            service_tier_override: None,
        })
        .await
        .unwrap();
    let mut tool_starts = 0;
    let mut failures = 0;
    let mut completions = 0;
    let mut kind = String::new();
    loop {
        match tokio::time::timeout(
            Duration::from_millis(if failures + completions > 0 {
                100
            } else {
                3_000
            }),
            events.recv(),
        )
        .await
        {
            Ok(Ok(event)) => {
                if event.kind == "tool.call_started" {
                    tool_starts += 1;
                }
                if event.kind == "turn.failed" {
                    failures += 1;
                    kind = serde_json::to_value(&event).unwrap().to_string();
                }
                if event.kind == "turn.completed" {
                    completions += 1;
                }
            }
            Err(_) if failures + completions > 0 => break,
            other => panic!("missing terminal: {other:?}"),
        }
    }
    let stops = std::fs::read_to_string(dir.path().join("stop.txt")).unwrap();
    assert_eq!(stops.lines().count(), 1, "Stop hook must run once");
    if matches!(mode, Mode::Replay(_)) {
        assert_eq!(tool_starts, 1, "completed call must not execute again");
    }
    (engine, failures, completions, kind)
}
#[tokio::test]
async fn non_retryable_typed_failures_have_one_terminal_and_stop() {
    for profile in [
        RuntimeProfile::Interactive,
        RuntimeProfile::NonInteractive,
        RuntimeProfile::Eval,
    ] {
        for kind in [
            ProviderFailureKind::Authentication,
            ProviderFailureKind::QuotaExceeded,
            ProviderFailureKind::UsageLimit,
            ProviderFailureKind::InvalidRequest,
            ProviderFailureKind::Protocol,
            ProviderFailureKind::ToolSearchExhausted,
            ProviderFailureKind::RequestTooLarge,
        ] {
            let (engine, failures, completed, event) =
                exercise_profile(Mode::Terminal(kind), profile).await;
            assert_eq!(engine.requests.lock().unwrap().len(), 1);
            assert_eq!((failures, completed), (1, 0));
            assert!(event.contains(kind.retry_cause()));
        }
    }
}
#[tokio::test]
async fn startup_retry_waits_for_provider_owned_cleanup() {
    let (engine, failed, completed, _) = exercise(Mode::SetupCleanup).await;
    assert_eq!(engine.requests.lock().unwrap().len(), 2);
    assert_eq!((failed, completed), (0, 1));
}
#[tokio::test]
async fn http_and_stream_retries_share_the_sampling_budget() {
    let (engine, failed, completed, _) = exercise(Mode::SharedBudget).await;
    let requests = engine.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1]
            .runtime
            .reliability
            .as_ref()
            .unwrap()
            .provider_retry_max_attempts,
        1
    );
    assert_eq!((failed, completed), (1, 0));
}
#[tokio::test]
async fn retry_budget_resets_after_completed_sampling_step() {
    let (engine, failed, completed, _) = exercise(Mode::ResetBudget).await;
    let requests = engine.requests.lock().unwrap();
    assert_eq!(requests.len(), 4);
    assert_eq!(
        requests
            .iter()
            .map(|request| request
                .runtime
                .reliability
                .as_ref()
                .unwrap()
                .provider_retry_max_attempts)
            .collect::<Vec<_>>(),
        vec![3, 2, 3, 2]
    );
    assert_eq!((failed, completed), (0, 1));
}

#[tokio::test]
async fn completed_call_replayed_in_later_sampling_does_not_repeat_execution() {
    let (engine, failed, completed, _) = exercise(Mode::Replay(false)).await;
    let requests = engine.requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert_eq!((failed, completed), (0, 1));
    assert_eq!(requests[2].transcript.iter().filter(|item|matches!(item,roder_api::transcript::TranscriptItem::ToolResult(result) if result.id=="once")).count(),1);
}
#[tokio::test]
async fn reused_completed_call_id_with_changed_arguments_fails_before_execution() {
    let (engine, failed, completed, event) = exercise(Mode::Replay(true)).await;
    assert_eq!(engine.requests.lock().unwrap().len(), 2);
    assert_eq!((failed, completed), (1, 0));
    assert!(event.contains("protocol"));
}
