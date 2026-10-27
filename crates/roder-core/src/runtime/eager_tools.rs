use super::*;
use roder_api::context::PolicyGate;
use tokio::task::JoinSet;

/// The turn owns these tasks; dropping the set aborts every unfinished read.
/// Reads must continue running while streamed item persistence awaits the same
/// transcript-store lock, so polling their futures only between items deadlocks.
pub(super) struct EagerTools {
    allowed: HashSet<String>,
    scheduled: HashSet<String>,
    pending: JoinSet<anyhow::Result<ToolResultRecord>>,
    results: HashMap<String, ToolResultRecord>,
    blocked_by_barrier: bool,
}

impl EagerTools {
    pub(super) fn new(
        runtime: &Runtime,
        specs: &[roder_api::tools::ToolSpec],
        external: &[roder_api::tools::ToolSpec],
        parallel: bool,
    ) -> Self {
        let exclusive_batch = specs
            .iter()
            .any(|spec| spec.name == roder_api::subagents::AGENT_SWARM_TOOL_NAME);
        let allowed = specs
            .iter()
            .filter(|spec| {
                parallel
                    && !exclusive_batch
                    && runtime.registry.policy_contributors.is_empty()
                    && !external.iter().any(|tool| tool.name == spec.name)
                    && runtime
                        .tool_registry
                        .get(&spec.name)
                        .is_some_and(|executor| {
                            executor.execution_mode()
                                == roder_api::tools::ToolExecutionMode::SharedRead
                        })
            })
            .map(|spec| spec.name.clone())
            .collect();
        Self {
            allowed,
            scheduled: HashSet::new(),
            pending: JoinSet::new(),
            results: HashMap::new(),
            blocked_by_barrier: false,
        }
    }

    pub(super) async fn schedule(
        &mut self,
        runtime: &Arc<Runtime>,
        context: &OutputContext<'_>,
        call: &ToolCallCompleted,
        workspace: &str,
        deadline: Option<OffsetDateTime>,
        transcript: &mut Vec<TranscriptItem>,
    ) -> anyhow::Result<()> {
        if self.scheduled.contains(&call.id) || self.blocked_by_barrier {
            return Ok(());
        }
        if !self.allowed.contains(&call.name) {
            // Never let a later streamed read overtake an earlier mutation.
            // Existing eager reads finish before ordinary dispatch begins.
            self.blocked_by_barrier = true;
            return Ok(());
        }
        let mode = runtime
            .effective_policy_mode_for_thread(context.thread_id)
            .await;
        let policy_context = roder_api::tools::ToolExecutionContext::new(
            context.thread_id.clone(),
            context.turn_id.clone(),
            mode,
        );
        let policy_call = roder_api::tools::ToolCall {
            thread_id: context.thread_id.clone(),
            turn_id: context.turn_id.clone(),
            id: call.id.clone(),
            name: call.name.clone(),
            arguments: serde_json::from_str(&call.arguments).unwrap_or_default(),
            raw_arguments: call.arguments.clone(),
        };
        if !matches!(
            crate::policy_gate::DefaultPolicyGate::new().decide(
                &policy_call,
                mode,
                &policy_context
            ),
            roder_api::policy_mode::PolicyDecision::Allowed
                | roder_api::policy_mode::PolicyDecision::AutoApproved { .. }
        ) {
            return Ok(());
        }
        self.scheduled.insert(call.id.clone());
        let item = TranscriptItem::ToolCall(ToolCallRecord {
            id: call.id.clone(),
            name: call.name.clone(),
            arguments: call.arguments.clone(),
        });
        runtime
            .persist_turn_item(context.thread_id, context.turn_id, &item)
            .await?;
        transcript.push(item);
        runtime
            .persist_model_profile_segment(
                context.thread_id,
                context.turn_id,
                context.profile,
                context.provider,
                context.model,
                "tool_call",
            )
            .await?;
        let runtime = runtime.clone();
        let thread_id = context.thread_id.clone();
        let turn_id = context.turn_id.clone();
        let workspace = workspace.to_string();
        let call = call.clone();
        self.pending.spawn(async move {
            runtime
                .route_tool_call(&thread_id, &turn_id, call, Some(&workspace), deadline, None)
                .await
        });
        Ok(())
    }

    pub(super) fn pending(&self) -> bool {
        !self.pending.is_empty()
    }
    pub(super) fn scheduled(&self, id: &str) -> bool {
        self.scheduled.contains(id)
    }

    pub(super) fn results_snapshot(&self, calls: &[ToolCallCompleted]) -> Vec<ToolResultRecord> {
        calls
            .iter()
            .filter_map(|call| self.results.get(&call.id).cloned())
            .collect()
    }

    pub(super) async fn poll_result(&mut self) -> anyhow::Result<()> {
        if let Some(result) = self.pending.join_next().await {
            let result = result??;
            self.results.insert(result.id.clone(), result);
        }
        Ok(())
    }

    pub(super) async fn drain(&mut self) -> anyhow::Result<()> {
        while self.pending() {
            self.poll_result().await?;
        }
        Ok(())
    }

    pub(super) fn take_results(&mut self, calls: &[ToolCallCompleted]) -> Vec<ToolResultRecord> {
        calls
            .iter()
            .filter_map(|call| self.results.remove(&call.id))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn eager_read_continues_while_stream_persistence_waits_for_its_lock() {
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        let started = Arc::new(tokio::sync::Notify::new());
        let mut eager = EagerTools {
            allowed: HashSet::new(),
            scheduled: HashSet::new(),
            pending: JoinSet::new(),
            results: HashMap::new(),
            blocked_by_barrier: false,
        };
        let read_lock = lock.clone();
        let read_started = started.clone();
        eager.pending.spawn(async move {
            let _guard = read_lock.lock().await;
            read_started.notify_one();
            tokio::time::sleep(Duration::from_millis(20)).await;
            Ok(ToolResultRecord {
                id: "read".into(),
                name: Some("read_file".into()),
                result: "contents".into(),
                display_payload: None,
                is_error: false,
            })
        });
        tokio::select! {
            _ = started.notified() => {},
            result = eager.poll_result() => panic!("read finished before holding the lock: {result:?}"),
        }
        // No poll_result here: processing the next stream item awaits storage.
        let guard = tokio::time::timeout(Duration::from_secs(1), lock.lock())
            .await
            .expect("eager read releases the lock without another sampling poll");
        drop(guard);
        eager.drain().await.unwrap();
        assert_eq!(eager.results["read"].result, "contents");
    }

    #[test]
    fn eager_reads_require_parallel_dispatch_and_no_swarm_or_external_shadow() {
        let mut registry = roder_api::extension::ExtensionRegistryBuilder::new();
        registry.inference_engine(Arc::new(crate::fake_provider::FakeInferenceEngine));
        registry.tool_contributor(Arc::new(roder_tools::EchoToolContributor));
        let runtime = Runtime::new(registry.build().unwrap(), RuntimeConfig::default()).unwrap();
        let spec = runtime.tool_registry.get("echo").unwrap().spec();
        assert!(
            EagerTools::new(&runtime, std::slice::from_ref(&spec), &[], true)
                .allowed
                .contains("echo")
        );
        assert!(
            EagerTools::new(&runtime, std::slice::from_ref(&spec), &[], false)
                .allowed
                .is_empty()
        );
        assert!(
            EagerTools::new(
                &runtime,
                std::slice::from_ref(&spec),
                std::slice::from_ref(&spec),
                true
            )
            .allowed
            .is_empty()
        );
        let swarm = roder_api::tools::ToolSpec {
            name: roder_api::subagents::AGENT_SWARM_TOOL_NAME.into(),
            description: "exclusive".into(),
            parameters: serde_json::json!({}),
        };
        assert!(
            EagerTools::new(&runtime, &[spec, swarm], &[], true)
                .allowed
                .is_empty()
        );
    }

    #[tokio::test]
    async fn streamed_read_cannot_overtake_an_exclusive_call() {
        let mut registry = roder_api::extension::ExtensionRegistryBuilder::new();
        registry.inference_engine(Arc::new(crate::fake_provider::FakeInferenceEngine));
        registry.tool_contributor(Arc::new(roder_tools::EchoToolContributor));
        let runtime =
            Arc::new(Runtime::new(registry.build().unwrap(), RuntimeConfig::default()).unwrap());
        let spec = runtime.tool_registry.get("echo").unwrap().spec();
        let mut eager = EagerTools::new(&runtime, &[spec], &[], true);
        let thread_id = "thread".to_string();
        let turn_id = "turn".to_string();
        let context = OutputContext {
            thread_id: &thread_id,
            turn_id: &turn_id,
            profile: None,
            provider: "mock",
            model: "mock",
        };
        let mut transcript = Vec::new();
        for (id, name) in [("mutation", "unknown_mutation"), ("read", "echo")] {
            eager
                .schedule(
                    &runtime,
                    &context,
                    &ToolCallCompleted {
                        id: id.into(),
                        name: name.into(),
                        arguments: "{}".into(),
                    },
                    ".",
                    None,
                    &mut transcript,
                )
                .await
                .unwrap();
        }
        assert!(eager.blocked_by_barrier);
        assert!(!eager.scheduled("read"));
        assert!(!eager.pending());
        assert!(transcript.is_empty(), "neither deferred call was executed");
    }
}
