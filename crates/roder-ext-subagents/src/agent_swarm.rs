//! Agent-swarm fanout (roadmap phase 104).
//!
//! This module adds the model-facing `agent_swarm` tool plus a bounded,
//! cancellation-aware scheduler that dispatches many homogeneous children
//! through the canonical [`SubagentDispatcher`] surface. It is a composition
//! layer: it never opens a second child-agent runtime, it only paces and
//! orders dispatch and renders an ordered, machine-readable result.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use roder_api::events::{ThreadId, TurnId};
use roder_api::inference::TokenUsage;
use roder_api::subagents::{
    AgentSwarmChildKind, AgentSwarmChildOutcome, AgentSwarmChildResult, AgentSwarmChildSpec,
    AgentSwarmChildState, AgentSwarmConfig, AgentSwarmRequest, AgentSwarmResult, SubagentDispatcher,
    SubagentExitReason, SubagentRequest, build_agent_swarm_specs,
};
use roder_api::tools::{
    ToolCall, ToolExecutionContext, ToolExecutionHandles, ToolExecutor, ToolResult, ToolSpec,
};
use roder_api::trace::SubagentTraceSink;
use serde_json::{Value, json};
use tokio::sync::{Notify, Semaphore};
use tokio::task::JoinSet;

/// Canonical model-facing swarm tool name (shared with `roder-api` and the
/// core turn loop's exclusivity enforcement).
pub use roder_api::subagents::AGENT_SWARM_TOOL_NAME as AGENT_SWARM_TOOL;

const AGENT_SWARM_DESCRIPTION: &str = "Launch many subagents from one prompt template, resume \
existing subagents, or both. Provide a `prompt_template` containing the exact placeholder \
`{{item}}` and an `items` array (at least two) whose values fill the placeholder, or pass \
`resume_agent_ids` mapping existing agent ids to continuation prompts. This tool must be the \
only tool call in the response; issue multiple swarms sequentially.";

/// Raw outcome of launching one child, before it is placed into ordered slots.
#[derive(Debug, Clone)]
pub struct AgentSwarmChildRun {
    pub agent_id: Option<String>,
    pub outcome: AgentSwarmChildOutcome,
    pub body: String,
    pub usage: Option<TokenUsage>,
}

/// Launches a single swarm child. Implementations must be infallible: dispatch
/// failures are reported as a `Failed` outcome, not as an `Err`, so the
/// scheduler can always return ordered results.
#[async_trait::async_trait]
pub trait AgentSwarmChildLauncher: Send + Sync {
    async fn launch(&self, spec: AgentSwarmChildSpec) -> AgentSwarmChildRun;
}

/// Cooperative cancellation handle for a running swarm. The tool path never
/// cancels (children are bounded by their own dispatch timeouts), but the
/// scheduler honors it so a future turn-interrupt wiring can stop fanout
/// cleanly without leaving children dangling.
#[derive(Clone)]
pub struct AgentSwarmCancel {
    flag: Arc<AtomicBool>,
    notify: Arc<Notify>,
}

impl Default for AgentSwarmCancel {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentSwarmCancel {
    pub fn new() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
            notify: Arc::new(Notify::new()),
        }
    }

    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        let notified = self.notify.notified();
        if self.is_cancelled() {
            return;
        }
        notified.await;
    }
}

enum ChildRunOrAbort {
    Run(AgentSwarmChildRun),
    Started,
    NotStarted,
}

/// Run a swarm: dispatch `specs` through `launcher` with a bounded normal-phase
/// ramp (initial burst, then one launch per interval), an optional concurrency
/// cap, ordered results, and cooperative cancellation.
pub async fn run_agent_swarm(
    launcher: Arc<dyn AgentSwarmChildLauncher>,
    specs: Vec<AgentSwarmChildSpec>,
    config: &AgentSwarmConfig,
    cancel: AgentSwarmCancel,
) -> AgentSwarmResult {
    let total = specs.len();
    if total == 0 {
        return AgentSwarmResult::from_children(Vec::new());
    }

    let config = config.clone().clamped();
    let cap = config.max_concurrency.unwrap_or(total).max(1);
    let concurrency = Arc::new(Semaphore::new(cap));

    // Launch credits gate how many children may *start* (the ramp). The initial
    // burst is available immediately; a pacer drips one more per interval.
    let initial = config.initial_launch_limit.min(total).max(1);
    let launch_gate = Arc::new(Semaphore::new(initial));
    let extra_needed = total.saturating_sub(initial);
    let interval = Duration::from_millis(config.launch_interval_ms);

    let pacer = if extra_needed > 0 {
        let gate = launch_gate.clone();
        Some(tokio::spawn(async move {
            for _ in 0..extra_needed {
                if interval.is_zero() {
                    tokio::task::yield_now().await;
                } else {
                    tokio::time::sleep(interval).await;
                }
                gate.add_permits(1);
            }
        }))
    } else {
        None
    };

    // On cancel, release every remaining launch credit at once so queued
    // children unblock immediately instead of waiting on the pacer.
    let watcher = {
        let gate = launch_gate.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move {
            cancel.cancelled().await;
            gate.add_permits(total);
        })
    };

    let mut set: JoinSet<(usize, ChildRunOrAbort)> = JoinSet::new();
    for (pos, spec) in specs.iter().cloned().enumerate() {
        let launcher = launcher.clone();
        let launch_gate = launch_gate.clone();
        let concurrency = concurrency.clone();
        let cancel = cancel.clone();
        set.spawn(async move {
            // Consume exactly one launch credit (permanently) to pace starts.
            match launch_gate.acquire().await {
                Ok(permit) => permit.forget(),
                Err(_) => return (pos, ChildRunOrAbort::NotStarted),
            }
            if cancel.is_cancelled() {
                return (pos, ChildRunOrAbort::NotStarted);
            }
            let _conc = match concurrency.acquire().await {
                Ok(permit) => permit,
                Err(_) => return (pos, ChildRunOrAbort::NotStarted),
            };
            if cancel.is_cancelled() {
                return (pos, ChildRunOrAbort::NotStarted);
            }
            tokio::select! {
                // Prefer cancellation when both are ready so an in-flight child
                // is reported aborted (started) rather than completing a turn
                // the user already interrupted.
                biased;
                _ = cancel.cancelled() => (pos, ChildRunOrAbort::Started),
                run = launcher.launch(spec) => (pos, ChildRunOrAbort::Run(run)),
            }
        });
    }

    let mut slots: Vec<Option<AgentSwarmChildResult>> = (0..total).map(|_| None).collect();
    while let Some(joined) = set.join_next().await {
        if let Ok((pos, outcome)) = joined {
            let spec = &specs[pos];
            slots[pos] = Some(match outcome {
                ChildRunOrAbort::Run(run) => child_result_from_run(spec, run),
                ChildRunOrAbort::Started => aborted_child(spec, AgentSwarmChildState::Started),
                ChildRunOrAbort::NotStarted => {
                    aborted_child(spec, AgentSwarmChildState::NotStarted)
                }
            });
        }
    }

    if let Some(pacer) = pacer {
        pacer.abort();
    }
    watcher.abort();

    let children = slots
        .into_iter()
        .map(|slot| slot.expect("every swarm child slot is filled"))
        .collect();
    AgentSwarmResult::from_children(children)
}

fn child_result_from_run(
    spec: &AgentSwarmChildSpec,
    run: AgentSwarmChildRun,
) -> AgentSwarmChildResult {
    AgentSwarmChildResult {
        index: spec.index,
        kind: spec.kind,
        item: spec.item.clone(),
        agent_id: run.agent_id,
        outcome: run.outcome,
        state: None,
        body: run.body,
        usage: run.usage,
    }
}

fn aborted_child(spec: &AgentSwarmChildSpec, state: AgentSwarmChildState) -> AgentSwarmChildResult {
    AgentSwarmChildResult {
        index: spec.index,
        kind: spec.kind,
        item: spec.item.clone(),
        agent_id: None,
        outcome: AgentSwarmChildOutcome::Aborted,
        state: Some(state),
        body: "aborted before completion".to_string(),
        usage: None,
    }
}

/// Maps a child swarm spec onto a canonical [`SubagentRequest`] and dispatches
/// it through a [`SubagentDispatcher`].
///
/// Note: the in-process dispatcher has no true "resume an existing agent id"
/// path yet, so resumed children are dispatched as fresh runs with the resume
/// prompt; the returned child thread id is surfaced as the `agent_id` so the
/// lead model can keep iterating.
pub struct DispatcherChildLauncher {
    dispatcher: Arc<dyn SubagentDispatcher>,
    parent_thread_id: ThreadId,
    parent_turn_id: TurnId,
    trace_sink: Option<Arc<dyn SubagentTraceSink>>,
    /// Parent turn handles (workspace, process runner, ...) so each child can
    /// operate on the same repository instead of failing with
    /// "workspace handle is not available".
    handles: ToolExecutionHandles,
    description: String,
    subagent_type: Option<String>,
    timeout_seconds: Option<u64>,
}

impl DispatcherChildLauncher {
    fn child_description(&self, spec: &AgentSwarmChildSpec) -> String {
        let label = match spec.kind {
            AgentSwarmChildKind::Resume => "resume".to_string(),
            AgentSwarmChildKind::Spawn => self
                .subagent_type
                .clone()
                .unwrap_or_else(|| "swarm".to_string()),
        };
        format!("{} #{} ({label})", self.description, spec.index)
    }
}

#[async_trait::async_trait]
impl AgentSwarmChildLauncher for DispatcherChildLauncher {
    async fn launch(&self, spec: AgentSwarmChildSpec) -> AgentSwarmChildRun {
        let request = SubagentRequest {
            description: self.child_description(&spec),
            prompt: spec.prompt.clone(),
            subagent_type: self.subagent_type.clone(),
            model: None,
            tools: None,
            lane: None,
            max_concurrent: None,
            allowed_tools: None,
            parent_deadline_seconds: None,
            inputs: None,
            timeout_seconds: self.timeout_seconds,
        };

        match self
            .dispatcher
            .dispatch_with_context(
                self.parent_thread_id.clone(),
                self.parent_turn_id.clone(),
                request,
                self.trace_sink.clone(),
                self.handles.clone(),
            )
            .await
        {
            Ok(result) => AgentSwarmChildRun {
                agent_id: Some(result.thread_id),
                outcome: outcome_for_exit(&result.exit_reason),
                body: result.final_message,
                usage: result.usage,
            },
            Err(err) => AgentSwarmChildRun {
                agent_id: None,
                outcome: AgentSwarmChildOutcome::Failed,
                body: err.to_string(),
                usage: None,
            },
        }
    }
}

fn outcome_for_exit(exit: &SubagentExitReason) -> AgentSwarmChildOutcome {
    match exit {
        SubagentExitReason::Completed => AgentSwarmChildOutcome::Completed,
        SubagentExitReason::Cancelled => AgentSwarmChildOutcome::Aborted,
        SubagentExitReason::MaxTurns
        | SubagentExitReason::Timeout
        | SubagentExitReason::Failed => AgentSwarmChildOutcome::Failed,
    }
}

/// The model-facing `agent_swarm` tool.
pub struct AgentSwarmTool {
    dispatcher: Arc<dyn SubagentDispatcher>,
    config: AgentSwarmConfig,
}

impl AgentSwarmTool {
    pub fn new(dispatcher: Arc<dyn SubagentDispatcher>, config: AgentSwarmConfig) -> Self {
        Self {
            dispatcher,
            config: config.clamped(),
        }
    }
}

#[async_trait::async_trait]
impl ToolExecutor for AgentSwarmTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: AGENT_SWARM_TOOL.to_string(),
            description: AGENT_SWARM_DESCRIPTION.to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "description": {
                        "type": "string",
                        "description": "Short description for the whole swarm."
                    },
                    "subagent_type": {
                        "type": "string",
                        "description": "Configured Roder subagent type used for every spawned child."
                    },
                    "prompt_template": {
                        "type": "string",
                        "description": "Prompt for each spawned child. The literal {{item}} placeholder is replaced with each item value."
                    },
                    "items": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Values that fill {{item}}; each launches one new child (at least two unless resume_agent_ids is provided)."
                    },
                    "resume_agent_ids": {
                        "type": "object",
                        "additionalProperties": { "type": "string" },
                        "description": "Map of existing subagent agent_id to the prompt used to resume it. Resumed children are launched before new ones."
                    }
                },
                "required": ["description"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(
        &self,
        ctx: ToolExecutionContext,
        call: ToolCall,
    ) -> anyhow::Result<ToolResult> {
        let request: AgentSwarmRequest = match serde_json::from_value(call.arguments) {
            Ok(request) => request,
            Err(err) => {
                return Ok(error_result(
                    call.id,
                    call.name,
                    "invalid_arguments",
                    err.to_string(),
                ));
            }
        };

        let specs = match build_agent_swarm_specs(&request, &self.config) {
            Ok(specs) => specs,
            Err(err) => {
                return Ok(error_result(
                    call.id,
                    call.name,
                    "invalid_arguments",
                    err.to_string(),
                ));
            }
        };

        let launcher: Arc<dyn AgentSwarmChildLauncher> = Arc::new(DispatcherChildLauncher {
            dispatcher: self.dispatcher.clone(),
            parent_thread_id: ctx.thread_id.clone(),
            parent_turn_id: ctx.turn_id.clone(),
            trace_sink: ctx.handles.subagent_trace_sink.clone(),
            handles: ctx.handles.clone(),
            description: request.description.clone(),
            subagent_type: request.subagent_type.clone(),
            timeout_seconds: self.config.child_timeout_seconds,
        });

        let result =
            run_agent_swarm(launcher, specs, &self.config, AgentSwarmCancel::new()).await;

        Ok(swarm_tool_result(call.id, call.name, result))
    }
}

fn swarm_tool_result(id: String, name: String, result: AgentSwarmResult) -> ToolResult {
    let summary = result.summary_line();
    let text = result.render_text();
    let data = json!({
        "summary": summary,
        "agent_swarm": serde_json::to_value(&result).unwrap_or(Value::Null),
    });
    ToolResult {
        id,
        name,
        text,
        data,
        is_error: false,
    }
}

fn error_result(id: String, name: String, kind: &'static str, message: String) -> ToolResult {
    ToolResult {
        id,
        name,
        text: message.clone(),
        data: json!({ "error": { "kind": kind, "message": message } }),
        is_error: true,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct RecordingLauncher {
        order: Mutex<Vec<usize>>,
        fail_indices: Vec<usize>,
        block_until_cancel: Option<AgentSwarmCancel>,
    }

    #[async_trait::async_trait]
    impl AgentSwarmChildLauncher for RecordingLauncher {
        async fn launch(&self, spec: AgentSwarmChildSpec) -> AgentSwarmChildRun {
            self.order.lock().unwrap().push(spec.index);
            if let Some(cancel) = &self.block_until_cancel {
                cancel.cancelled().await;
            }
            if self.fail_indices.contains(&spec.index) {
                AgentSwarmChildRun {
                    agent_id: Some(format!("agent-{}", spec.index)),
                    outcome: AgentSwarmChildOutcome::Failed,
                    body: format!("child {} failed", spec.index),
                    usage: None,
                }
            } else {
                AgentSwarmChildRun {
                    agent_id: Some(format!("agent-{}", spec.index)),
                    outcome: AgentSwarmChildOutcome::Completed,
                    body: format!("child {} done", spec.index),
                    usage: None,
                }
            }
        }
    }

    fn spec(index: usize, kind: AgentSwarmChildKind) -> AgentSwarmChildSpec {
        AgentSwarmChildSpec {
            index,
            kind,
            item: Some(format!("item-{index}")),
            prompt: format!("prompt {index}"),
            resume_agent_id: None,
        }
    }

    fn fast_config() -> AgentSwarmConfig {
        AgentSwarmConfig {
            max_subagents: 128,
            initial_launch_limit: 5,
            launch_interval_ms: 0,
            max_concurrency: None,
            child_timeout_seconds: None,
        }
    }

    #[tokio::test]
    async fn returns_results_in_input_order() {
        let launcher = Arc::new(RecordingLauncher::default());
        let specs = vec![
            spec(1, AgentSwarmChildKind::Spawn),
            spec(2, AgentSwarmChildKind::Spawn),
            spec(3, AgentSwarmChildKind::Spawn),
        ];
        let result =
            run_agent_swarm(launcher, specs, &fast_config(), AgentSwarmCancel::new()).await;
        assert_eq!(result.completed, 3);
        let indices: Vec<usize> = result.children.iter().map(|c| c.index).collect();
        assert_eq!(indices, vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn reports_failures_with_agent_ids_for_resume() {
        let launcher = Arc::new(RecordingLauncher {
            fail_indices: vec![2],
            ..RecordingLauncher::default()
        });
        let specs = vec![
            spec(1, AgentSwarmChildKind::Spawn),
            spec(2, AgentSwarmChildKind::Spawn),
        ];
        let result =
            run_agent_swarm(launcher, specs, &fast_config(), AgentSwarmCancel::new()).await;
        assert_eq!(result.completed, 1);
        assert_eq!(result.failed, 1);
        assert!(result.needs_resume_hint());
        assert_eq!(result.children[1].agent_id.as_deref(), Some("agent-2"));
    }

    #[tokio::test]
    async fn max_concurrency_one_serializes_launches() {
        let launcher = Arc::new(RecordingLauncher::default());
        let specs: Vec<_> = (1..=4)
            .map(|i| spec(i, AgentSwarmChildKind::Spawn))
            .collect();
        let config = AgentSwarmConfig {
            max_concurrency: Some(1),
            ..fast_config()
        };
        let result = run_agent_swarm(launcher.clone(), specs, &config, AgentSwarmCancel::new()).await;
        assert_eq!(result.completed, 4);
        // With a concurrency cap of one the children execute one at a time in
        // input order.
        assert_eq!(*launcher.order.lock().unwrap(), vec![1, 2, 3, 4]);
    }

    /// A launcher that proves true parallel overlap: every child must be
    /// simultaneously active to clear the barrier, so the recorded peak equals
    /// the expected concurrency. This is the offline evidence (roadmap 104,
    /// Task 6) that the swarm scheduler runs independent children in parallel
    /// rather than serially, with no flaky sleeps.
    struct OverlapLauncher {
        barrier: Arc<tokio::sync::Barrier>,
    }

    #[async_trait::async_trait]
    impl AgentSwarmChildLauncher for OverlapLauncher {
        async fn launch(&self, spec: AgentSwarmChildSpec) -> AgentSwarmChildRun {
            // Blocks until `barrier` count children are concurrently here.
            self.barrier.wait().await;
            AgentSwarmChildRun {
                agent_id: Some(format!("agent-{}", spec.index)),
                outcome: AgentSwarmChildOutcome::Completed,
                body: "done".to_string(),
                usage: None,
            }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn independent_children_run_in_parallel() {
        // Four children, initial burst of 5, no cap: all four must be active at
        // once to pass a 4-way barrier — deterministic proof of overlap.
        let launcher = Arc::new(OverlapLauncher {
            barrier: Arc::new(tokio::sync::Barrier::new(4)),
        });
        let specs: Vec<_> = (1..=4)
            .map(|i| spec(i, AgentSwarmChildKind::Spawn))
            .collect();
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            run_agent_swarm(launcher, specs, &fast_config(), AgentSwarmCancel::new()),
        )
        .await
        .expect("parallel swarm should not deadlock");
        assert_eq!(result.completed, 4);
    }

    #[tokio::test]
    async fn cancellation_marks_unfinished_children_aborted() {
        let cancel = AgentSwarmCancel::new();
        let launcher = Arc::new(RecordingLauncher {
            block_until_cancel: Some(cancel.clone()),
            ..RecordingLauncher::default()
        });
        let specs: Vec<_> = (1..=3)
            .map(|i| spec(i, AgentSwarmChildKind::Spawn))
            .collect();
        let config = fast_config();
        let handle = {
            let cancel = cancel.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(20)).await;
                cancel.cancel();
            })
        };
        let result = run_agent_swarm(launcher, specs, &config, cancel).await;
        handle.await.unwrap();
        assert_eq!(result.completed, 0);
        assert_eq!(result.aborted, 3);
        assert!(
            result
                .children
                .iter()
                .all(|child| child.outcome == AgentSwarmChildOutcome::Aborted)
        );
    }
}
