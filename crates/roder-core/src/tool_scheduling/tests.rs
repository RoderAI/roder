use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use roder_api::extension::{ExtensionRegistryBuilder, ToolProviderId};
use roder_api::tools::{
    ToolCall, ToolContributor, ToolExecutionBarrier as ToolScheduler, ToolExecutionContext,
    ToolExecutor, ToolRegistry, ToolResult, ToolSpec,
};
use roder_ext_jsonl_thread_store::store::JsonlThreadStoreFactory;
use serde_json::json;
use tokio::sync::Barrier;

use super::*;
use crate::runtime::RuntimeConfig;

#[tokio::test]
async fn readers_overlap_and_writer_excludes_readers_and_writers() {
    let scheduler = Arc::new(ToolScheduler::default());
    let rendezvous = Arc::new(Barrier::new(3));
    let release = Arc::new(Barrier::new(3));
    let mut readers = Vec::new();
    for _ in 0..2 {
        let scheduler = scheduler.clone();
        let rendezvous = rendezvous.clone();
        let release = release.clone();
        readers.push(tokio::spawn(async move {
            let _guard = scheduler.acquire(ToolExecutionMode::SharedRead).await;
            rendezvous.wait().await;
            release.wait().await;
        }));
    }
    tokio::time::timeout(Duration::from_secs(2), rendezvous.wait())
        .await
        .expect("both readers must acquire the barrier concurrently");
    let writer = scheduler.acquire(ToolExecutionMode::Exclusive);
    tokio::pin!(writer);
    assert!(futures::poll!(&mut writer).is_pending());
    release.wait().await;
    let guard = tokio::time::timeout(Duration::from_secs(2), writer)
        .await
        .unwrap();
    let mut other_writer = Box::pin(scheduler.acquire(ToolExecutionMode::Exclusive));
    let mut read = Box::pin(scheduler.acquire(ToolExecutionMode::SharedRead));
    assert!(futures::poll!(&mut other_writer).is_pending());
    assert!(futures::poll!(&mut read).is_pending());
    drop(guard);
    // Cancel the queued writer so the read can acquire.
    drop(other_writer);
    tokio::time::timeout(Duration::from_secs(2), read)
        .await
        .expect("dropping a waiting writer must release the queue");
    for reader in readers {
        reader.await.unwrap();
    }
}

#[tokio::test]
async fn cancellation_releases_held_and_waiting_barriers() {
    let scheduler = Arc::new(ToolScheduler::default());
    let entered = Arc::new(tokio::sync::Notify::new());
    let task = {
        let scheduler = scheduler.clone();
        let entered = entered.clone();
        tokio::spawn(async move {
            let _guard = scheduler.acquire(ToolExecutionMode::Exclusive).await;
            entered.notify_one();
            futures::future::pending::<()>().await;
        })
    };
    entered.notified().await;
    let mut waiter = Box::pin(scheduler.acquire(ToolExecutionMode::Exclusive));
    assert!(futures::poll!(&mut waiter).is_pending());
    drop(waiter);
    task.abort();
    let _ = task.await;
    tokio::time::timeout(
        Duration::from_secs(2),
        scheduler.acquire(ToolExecutionMode::SharedRead),
    )
    .await
    .expect("aborted tasks must not retain scheduling guards");
}

#[derive(Default)]
struct ProbeState {
    readers: AtomicUsize,
    writers: AtomicUsize,
    log: Mutex<Vec<String>>,
}

struct ReadProbe {
    state: Arc<ProbeState>,
    pair: Arc<Barrier>,
    hold: Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>,
}

fn spec(name: &str) -> ToolSpec {
    ToolSpec {
        name: name.into(),
        description: "scheduling test".into(),
        parameters: json!({
            "type": "object",
            "properties": {"pair": {"type": "boolean"}},
            "required": ["pair"],
            "additionalProperties": false
        }),
    }
}

fn result(call: ToolCall) -> ToolResult {
    ToolResult {
        id: call.id,
        name: call.name,
        text: "ok".into(),
        data: json!({}),
        is_error: false,
    }
}

#[async_trait::async_trait]
impl ToolExecutor for ReadProbe {
    fn spec(&self) -> ToolSpec {
        spec("read_probe")
    }

    fn execution_mode(&self) -> ToolExecutionMode {
        ToolExecutionMode::SharedRead
    }

    async fn execute(&self, _: ToolExecutionContext, call: ToolCall) -> anyhow::Result<ToolResult> {
        assert_eq!(self.state.writers.load(Ordering::SeqCst), 0);
        self.state.readers.fetch_add(1, Ordering::SeqCst);
        self.state
            .log
            .lock()
            .unwrap()
            .push(format!("{}:start", call.id));
        if let Some((started, release)) = &self.hold {
            started.notify_one();
            release.notified().await;
        }
        if call.arguments["pair"] == true {
            self.pair.wait().await;
            assert_eq!(self.state.writers.load(Ordering::SeqCst), 0);
        }
        self.state
            .log
            .lock()
            .unwrap()
            .push(format!("{}:end", call.id));
        self.state.readers.fetch_sub(1, Ordering::SeqCst);
        Ok(result(call))
    }
}

/// Deliberately does not opt in: an unknown executor must remain exclusive.
struct UnknownProbe(Arc<ProbeState>);

#[async_trait::async_trait]
impl ToolExecutor for UnknownProbe {
    fn spec(&self) -> ToolSpec {
        spec("unknown_probe")
    }

    async fn execute(&self, _: ToolExecutionContext, call: ToolCall) -> anyhow::Result<ToolResult> {
        assert_eq!(self.0.readers.load(Ordering::SeqCst), 0);
        assert_eq!(self.0.writers.fetch_add(1, Ordering::SeqCst), 0);
        self.0
            .log
            .lock()
            .unwrap()
            .push(format!("{}:start", call.id));
        tokio::task::yield_now().await;
        assert_eq!(self.0.readers.load(Ordering::SeqCst), 0);
        self.0.log.lock().unwrap().push(format!("{}:end", call.id));
        self.0.writers.fetch_sub(1, Ordering::SeqCst);
        Ok(result(call))
    }
}

struct CoordinatingProbe;

#[async_trait::async_trait]
impl ToolExecutor for CoordinatingProbe {
    fn spec(&self) -> ToolSpec {
        spec("coordinator_probe")
    }

    fn execution_mode(&self) -> ToolExecutionMode {
        ToolExecutionMode::Coordinator
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let barrier = ctx.handles.execution_barrier.as_ref().unwrap();
        let _child_guard = barrier.acquire(ToolExecutionMode::Exclusive).await;
        Ok(result(call))
    }
}

struct Probes(Vec<Arc<dyn ToolExecutor>>);

impl ToolContributor for Probes {
    fn id(&self) -> ToolProviderId {
        "scheduling-probes".into()
    }

    fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
        for tool in &self.0 {
            registry.register(tool.clone())?;
        }
        Ok(())
    }
}

fn call(id: &str, name: &str, pair: bool) -> ToolCallCompleted {
    ToolCallCompleted {
        id: id.into(),
        name: name.into(),
        arguments: json!({"pair": pair}).to_string(),
    }
}

struct DenyRead(Arc<AtomicUsize>);

#[async_trait::async_trait]
impl roder_api::context::PolicyContributor for DenyRead {
    fn id(&self) -> roder_api::extension::PolicyContributorId {
        "deny-scheduling-probe".into()
    }

    async fn review_tool(
        &self,
        _: roder_api::context::PolicyReview,
    ) -> anyhow::Result<roder_api::context::PolicyContribution> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(roder_api::context::PolicyContribution::Deny {
            reason: "a scheduling hint does not grant permission".into(),
        })
    }
}

#[tokio::test]
async fn shared_read_metadata_does_not_bypass_policy_review() {
    let directory = tempfile::tempdir().unwrap();
    let state = Arc::new(ProbeState::default());
    let reviews = Arc::new(AtomicUsize::new(0));
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(crate::fake_provider::FakeInferenceEngine));
    builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
        base_path: directory.path().to_path_buf(),
    }));
    builder.tool_contributor(Arc::new(Probes(vec![Arc::new(ReadProbe {
        state: state.clone(),
        pair: Arc::new(Barrier::new(1)),
        hold: None,
    })])));
    builder.policy_contributor(Arc::new(DenyRead(reviews.clone())));
    let config = RuntimeConfig {
        policy_mode: roder_api::policy_mode::PolicyMode::Bypass,
        tool_allowlist: vec!["read_probe".into()],
        ..RuntimeConfig::default()
    };
    let runtime = Arc::new(Runtime::new(builder.build().unwrap(), config).unwrap());
    let thread = runtime.create_thread(None).await.unwrap().thread_id;
    let result = runtime
        .route_tool_call(
            &thread,
            &"policy-turn".into(),
            call("read", "read_probe", false),
            None,
            None,
            None,
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(
        result
            .result
            .contains("scheduling hint does not grant permission")
    );
    assert_eq!(reviews.load(Ordering::SeqCst), 1);
    assert!(state.log.lock().unwrap().is_empty());
}

#[tokio::test]
async fn runtime_batches_adjacent_reads_without_crossing_mutations() {
    let directory = tempfile::tempdir().unwrap();
    let state = Arc::new(ProbeState::default());
    let unknown = Arc::new(UnknownProbe(state.clone()));
    assert_eq!(unknown.execution_mode(), ToolExecutionMode::Exclusive);
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(crate::fake_provider::FakeInferenceEngine));
    builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
        base_path: directory.path().to_path_buf(),
    }));
    builder.tool_contributor(Arc::new(Probes(vec![
        Arc::new(ReadProbe {
            state: state.clone(),
            pair: Arc::new(Barrier::new(2)),
            hold: None,
        }),
        unknown,
        Arc::new(CoordinatingProbe),
    ])));
    let config = RuntimeConfig {
        policy_mode: roder_api::policy_mode::PolicyMode::Bypass,
        tool_allowlist: vec![
            "read_probe".into(),
            "unknown_probe".into(),
            "coordinator_probe".into(),
        ],
        ..RuntimeConfig::default()
    };
    let runtime = Arc::new(Runtime::new(builder.build().unwrap(), config).unwrap());
    let thread = runtime.create_thread(None).await.unwrap().thread_id;
    assert_eq!(
        runtime.tool_execution_mode("unregistered"),
        ToolExecutionMode::Exclusive
    );
    let results = tokio::time::timeout(
        Duration::from_secs(5),
        runtime.execute_scheduled_tools(
            &thread,
            &"scheduling-turn".into(),
            vec![
                call("a", "read_probe", true),
                call("b", "read_probe", true),
                call("w", "unknown_probe", false),
                call("w2", "unknown_probe", false),
                call("c", "read_probe", false),
            ],
            true,
            None,
            None,
        ),
    )
    .await
    .expect("shared reads must overlap, not deadlock")
    .unwrap();
    assert_eq!(
        results
            .iter()
            .map(|result| result.id.as_str())
            .collect::<Vec<_>>(),
        ["a", "b", "w", "w2", "c"]
    );
    assert!(results.iter().all(|result| !result.is_error), "{results:?}");
    let log = state.log.lock().unwrap();
    let index = |entry| log.iter().position(|value| value == entry).unwrap();
    assert!(index("a:end") < index("w:start"));
    assert!(index("b:end") < index("w:start"));
    assert!(index("w:end") < index("w2:start"));
    assert!(index("w2:end") < index("c:start"));
    drop(log);
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        runtime.route_tool_call(
            &thread,
            &"scheduling-turn".into(),
            call("nested", "coordinator_probe", false),
            None,
            None,
            None,
        ),
    )
    .await
    .expect("a coordinator must not hold the barrier while awaiting a child")
    .unwrap();
    assert!(!result.is_error, "{result:?}");
}

#[tokio::test]
async fn independent_runtime_dispatches_share_the_same_resource_barrier() {
    let directory = tempfile::tempdir().unwrap();
    let state = Arc::new(ProbeState::default());
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(crate::fake_provider::FakeInferenceEngine));
    builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
        base_path: directory.path().to_path_buf(),
    }));
    builder.tool_contributor(Arc::new(Probes(vec![
        Arc::new(ReadProbe {
            state: state.clone(),
            pair: Arc::new(Barrier::new(1)),
            hold: Some((started.clone(), release.clone())),
        }),
        Arc::new(UnknownProbe(state)),
    ])));
    let runtime = Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                policy_mode: roder_api::policy_mode::PolicyMode::Bypass,
                tool_allowlist: vec!["read_probe".into(), "unknown_probe".into()],
                ..RuntimeConfig::default()
            },
        )
        .unwrap(),
    );
    let thread = runtime.create_thread(None).await.unwrap().thread_id;
    let read = tokio::spawn({
        let runtime = runtime.clone();
        let thread = thread.clone();
        async move {
            runtime
                .route_tool_call(
                    &thread,
                    &"read-turn".into(),
                    call("read", "read_probe", false),
                    None,
                    None,
                    None,
                )
                .await
        }
    });
    tokio::time::timeout(Duration::from_secs(2), started.notified())
        .await
        .unwrap();
    let mut write = tokio::spawn({
        let runtime = runtime.clone();
        async move {
            runtime
                .route_tool_call(
                    &thread,
                    &"write-turn".into(),
                    call("write", "unknown_probe", false),
                    None,
                    None,
                    None,
                )
                .await
        }
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut write)
            .await
            .is_err(),
        "a separately dispatched writer must wait for the active read"
    );
    release.notify_one();
    assert!(
        !tokio::time::timeout(Duration::from_secs(2), read)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
            .is_error
    );
    assert!(
        !tokio::time::timeout(Duration::from_secs(2), write)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
            .is_error
    );
}
