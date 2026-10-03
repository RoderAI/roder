//! Local execution authority for a durable hosted-runtime owner. The supervisor
//! extends this deadline only after a confirmed renewal of the same generation.
//! Durable writes and remote/browser actions still require their own fencing.
use crate::Runtime;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Instant,
};

#[derive(Debug)]
pub struct RuntimeExecutionLease {
    deadline: Mutex<Option<Instant>>,
    executions: AtomicUsize,
}

impl RuntimeExecutionLease {
    /// Use request-start + TTL, never a database wall-clock timestamp. If the
    /// request took too long, this deadline can already be expired and fails closed.
    pub fn new(deadline: Instant) -> Self {
        Self {
            deadline: Mutex::new(Some(deadline)),
            executions: AtomicUsize::new(0),
        }
    }

    /// A failed or late renewal cannot resurrect a runtime that lost authority.
    pub fn renew(&self, deadline: Instant) -> anyhow::Result<()> {
        let mut current = self
            .deadline
            .lock()
            .map_err(|_| anyhow::anyhow!("runtime execution lease unavailable"))?;
        let now = Instant::now();
        match *current {
            Some(existing) if existing > now && deadline > now => {
                *current = Some(existing.max(deadline));
                Ok(())
            }
            _ => {
                *current = None;
                anyhow::bail!("runtime execution lease expired or revoked")
            }
        }
    }

    pub(crate) fn enter(self: &Arc<Self>) -> anyhow::Result<ExecutionPermit> {
        let mut deadline = self
            .deadline
            .lock()
            .map_err(|_| anyhow::anyhow!("runtime execution lease unavailable"))?;
        if !deadline.is_some_and(|deadline| deadline > Instant::now()) {
            *deadline = None;
            anyhow::bail!("runtime execution lease expired or revoked");
        }
        self.executions.fetch_add(1, Ordering::AcqRel);
        Ok(ExecutionPermit(self.clone()))
    }

    pub(crate) fn seal_if_idle(&self) -> anyhow::Result<bool> {
        let mut deadline = self
            .deadline
            .lock()
            .map_err(|_| anyhow::anyhow!("runtime execution lease unavailable"))?;
        anyhow::ensure!(
            deadline.is_some_and(|deadline| deadline > Instant::now()),
            "runtime execution lease expired or revoked"
        );
        if self.executions.load(Ordering::Acquire) != 0 {
            return Ok(false);
        }
        *deadline = None;
        Ok(true)
    }

    pub fn revoke(&self) {
        if let Ok(mut deadline) = self.deadline.lock() {
            *deadline = None;
        }
    }

    pub fn require_live(&self) -> anyhow::Result<()> {
        let mut deadline = self
            .deadline
            .lock()
            .map_err(|_| anyhow::anyhow!("runtime execution lease unavailable"))?;
        if deadline.is_some_and(|deadline| deadline > Instant::now()) {
            return Ok(());
        }
        *deadline = None;
        anyhow::bail!("runtime execution lease expired or revoked")
    }
}

/// Counts local tool futures, including approval and cleanup waits. A successful
/// planned handoff cannot race a new dispatch or abandon an admitted future.
pub(crate) struct ExecutionPermit(Arc<RuntimeExecutionLease>);
impl Drop for ExecutionPermit {
    fn drop(&mut self) {
        self.0.executions.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Runtime {
    /// Bind before publishing the runtime or accepting work. Replacing a lost
    /// generation requires constructing a new runtime, not resetting this guard.
    pub fn with_execution_lease(mut self, lease: Arc<RuntimeExecutionLease>) -> Self {
        self.execution_lease = Some(lease);
        self
    }

    pub fn uses_execution_lease(&self, lease: &Arc<RuntimeExecutionLease>) -> bool {
        self.execution_lease
            .as_ref()
            .is_some_and(|bound| Arc::ptr_eq(bound, lease))
    }

    pub fn ensure_execution_authority(&self) -> anyhow::Result<()> {
        if let Some(lease) = &self.execution_lease {
            lease.require_live()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RuntimeConfig, StartTurnRequest, fake_provider::FakeInferenceEngine};
    use roder_api::{extension::ExtensionRegistryBuilder, tools::*};
    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    };

    #[test]
    fn late_renewal_and_revocation_cannot_restore_authority() {
        let expired = RuntimeExecutionLease::new(Instant::now());
        assert!(
            expired
                .renew(Instant::now() + Duration::from_secs(30))
                .is_err()
        );
        let live = RuntimeExecutionLease::new(Instant::now() + Duration::from_secs(30));
        live.renew(Instant::now() + Duration::from_secs(60))
            .unwrap();
        live.require_live().unwrap();
        live.revoke();
        assert!(live.require_live().is_err());
        assert!(
            live.renew(Instant::now() + Duration::from_secs(60))
                .is_err()
        );
    }

    struct Probe(Arc<AtomicUsize>);
    impl ToolContributor for Probe {
        fn id(&self) -> String {
            "lease-probe".into()
        }
        fn contribute(&self, registry: &mut ToolRegistry) -> anyhow::Result<()> {
            registry.register(Arc::new(Probe(self.0.clone())))
        }
    }
    #[async_trait::async_trait]
    impl ToolExecutor for Probe {
        fn spec(&self) -> ToolSpec {
            ToolSpec {
                name: "write_file".into(),
                description: "probe".into(),
                parameters: serde_json::json!({"type":"object","properties":{}}),
            }
        }
        async fn execute(
            &self,
            _ctx: ToolExecutionContext,
            call: ToolCall,
        ) -> anyhow::Result<ToolResult> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(ToolResult {
                id: call.id,
                name: call.name,
                text: "ok".into(),
                data: serde_json::json!({}),
                is_error: false,
            })
        }
    }

    #[tokio::test]
    async fn revoked_owner_cannot_start_turns_or_execute_more_tools() {
        let count = Arc::new(AtomicUsize::new(0));
        let lease = Arc::new(RuntimeExecutionLease::new(
            Instant::now() + Duration::from_secs(30),
        ));
        let mut registry = ExtensionRegistryBuilder::new();
        registry.inference_engine(Arc::new(FakeInferenceEngine));
        registry.tool_contributor(Arc::new(Probe(count.clone())));
        let runtime = Arc::new(
            Runtime::new(
                registry.build().unwrap(),
                RuntimeConfig {
                    policy_mode: roder_api::policy_mode::PolicyMode::Bypass,
                    ..Default::default()
                },
            )
            .unwrap()
            .with_execution_lease(lease.clone()),
        );
        let thread = runtime.create_thread(None).await.unwrap().thread_id;
        let call = || roder_api::inference::ToolCallCompleted {
            id: uuid::Uuid::new_v4().to_string(),
            name: "write_file".into(),
            arguments: "{}".into(),
        };
        runtime
            .route_tool_call(&thread, &"turn".into(), call(), None, None)
            .await
            .unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 1);
        lease.revoke();
        assert!(
            runtime
                .route_tool_call(&thread, &"turn".into(), call(), None, None)
                .await
                .unwrap_err()
                .to_string()
                .contains("execution lease")
        );
        assert_eq!(count.load(Ordering::SeqCst), 1);
        let error = runtime
            .start_turn(StartTurnRequest {
                thread_id: thread,
                message: "do work".into(),
                images: Vec::new(),
                provider_override: None,
                model_override: None,
                reasoning_override: None,
                workspace: std::env::current_dir().unwrap().display().to_string(),
                instructions: crate::default_instructions(),
                developer_context: None,
                task_ledger_required: false,
                service_tier_override: None,
            })
            .await
            .unwrap_err();
        assert!(error.to_string().contains("execution lease"));
    }
    #[tokio::test]
    async fn ownership_is_rechecked_after_a_user_approval_wait() {
        let count = Arc::new(AtomicUsize::new(0));
        let lease = Arc::new(RuntimeExecutionLease::new(
            Instant::now() + Duration::from_secs(30),
        ));
        let mut registry = ExtensionRegistryBuilder::new();
        registry.inference_engine(Arc::new(FakeInferenceEngine));
        registry.tool_contributor(Arc::new(Probe(count.clone())));
        let runtime = Arc::new(
            Runtime::new(registry.build().unwrap(), RuntimeConfig::default())
                .unwrap()
                .with_execution_lease(lease.clone()),
        );
        let thread = runtime.create_thread(None).await.unwrap().thread_id;
        let mut events = runtime.subscribe_events();
        let task_runtime = runtime.clone();
        let task = tokio::spawn(async move {
            task_runtime
                .route_tool_call(
                    &thread,
                    &"turn".into(),
                    roder_api::inference::ToolCallCompleted {
                        id: "approval-lease-call".into(),
                        name: "write_file".into(),
                        arguments: "{}".into(),
                    },
                    None,
                    None,
                )
                .await
        });
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if matches!(
                    events.recv().await.unwrap().event,
                    roder_api::events::RoderEvent::ApprovalRequested(_)
                ) {
                    break;
                }
            }
        })
        .await
        .unwrap();
        lease.revoke();
        assert!(
            runtime
                .resolve_tool_approval("approval-lease-call", true)
                .await
                .unwrap()
        );
        let result = tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(result.is_error);
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }
    #[tokio::test]
    async fn handoff_waits_for_approved_tool_to_finish_without_interrupting_it() {
        let count = Arc::new(AtomicUsize::new(0));
        let lease = Arc::new(RuntimeExecutionLease::new(
            Instant::now() + Duration::from_secs(30),
        ));
        let mut registry = ExtensionRegistryBuilder::new();
        registry.inference_engine(Arc::new(FakeInferenceEngine));
        registry.tool_contributor(Arc::new(Probe(count.clone())));
        let runtime = Arc::new(
            Runtime::new(registry.build().unwrap(), RuntimeConfig::default())
                .unwrap()
                .with_execution_lease(lease.clone()),
        );
        let thread = runtime.create_thread(None).await.unwrap().thread_id;
        let mut events = runtime.subscribe_events();
        let task_runtime = runtime.clone();
        let task = tokio::spawn(async move {
            task_runtime
                .route_tool_call(
                    &thread,
                    &"turn".into(),
                    roder_api::inference::ToolCallCompleted {
                        id: "approval-lease-call".into(),
                        name: "write_file".into(),
                        arguments: "{}".into(),
                    },
                    None,
                    None,
                )
                .await
        });
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if matches!(
                    events.recv().await.unwrap().event,
                    roder_api::events::RoderEvent::ApprovalRequested(_)
                ) {
                    break;
                }
            }
        })
        .await
        .unwrap();
        assert!(!runtime.seal_idle_owner().await.unwrap());
        lease.require_live().unwrap();
        assert!(
            runtime
                .resolve_tool_approval("approval-lease-call", true)
                .await
                .unwrap()
        );
        let result = tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(!result.is_error);
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert!(runtime.seal_idle_owner().await.unwrap());
        assert!(lease.require_live().is_err());
    }
    #[tokio::test]
    async fn handoff_leaves_an_active_provider_turn_running_until_completion() {
        use roder_api::inference::*;
        struct WaitingEngine {
            entered: tokio::sync::Notify,
            release: tokio::sync::Notify,
        }
        #[async_trait::async_trait]
        impl InferenceEngine for WaitingEngine {
            fn id(&self) -> String {
                "mock".into()
            }
            fn capabilities(&self) -> InferenceCapabilities {
                InferenceCapabilities::text_only()
            }
            async fn list_models(
                &self,
                _: InferenceProviderContext<'_>,
            ) -> anyhow::Result<Vec<ModelDescriptor>> {
                Ok(vec![])
            }
            async fn stream_turn(
                &self,
                _: InferenceTurnContext<'_>,
                _: AgentInferenceRequest,
            ) -> anyhow::Result<InferenceEventStream> {
                self.entered.notify_one();
                self.release.notified().await;
                Ok(Box::pin(futures::stream::iter([Ok(
                    InferenceEvent::Completed(CompletionMetadata {
                        stop_reason: Some("completed".into()),
                        provider_response_id: None,
                    }),
                )])))
            }
        }
        let engine = Arc::new(WaitingEngine {
            entered: Default::default(),
            release: Default::default(),
        });
        let lease = Arc::new(RuntimeExecutionLease::new(
            Instant::now() + Duration::from_secs(30),
        ));
        let mut registry = ExtensionRegistryBuilder::new();
        registry.inference_engine(engine.clone());
        let runtime = Arc::new(
            Runtime::new(registry.build().unwrap(), RuntimeConfig::default())
                .unwrap()
                .with_execution_lease(lease.clone()),
        );
        let thread = runtime.create_thread(None).await.unwrap().thread_id;
        runtime
            .start_turn(StartTurnRequest {
                thread_id: thread,
                message: "finish normally".into(),
                images: vec![],
                provider_override: None,
                model_override: None,
                reasoning_override: None,
                workspace: std::env::current_dir().unwrap().display().to_string(),
                instructions: crate::default_instructions(),
                developer_context: None,
                task_ledger_required: false,
                service_tier_override: None,
            })
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), engine.entered.notified())
            .await
            .unwrap();
        assert!(!runtime.seal_idle_owner().await.unwrap());
        assert_eq!(runtime.active_turn_count().await, 1);
        lease.require_live().unwrap();
        engine.release.notify_one();
        tokio::time::timeout(Duration::from_secs(3), async {
            while !runtime.seal_idle_owner().await.unwrap() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(runtime.active_turn_count().await, 0);
        assert!(lease.require_live().is_err());
    }
}
