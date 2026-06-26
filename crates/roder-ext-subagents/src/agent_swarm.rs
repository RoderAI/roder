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
    AgentSwarmChildState, AgentSwarmConfig, AgentSwarmProgressSink, AgentSwarmProgressSnapshot,
    AgentSwarmRequest, AgentSwarmResult, SubagentDispatcher, SubagentExitReason, SubagentRequest,
    build_agent_swarm_specs,
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
    /// True when the child failed due to a provider rate limit. The scheduler
    /// uses this to requeue the child with backoff instead of failing it
    /// outright.
    pub rate_limited: bool,
}

/// Launches a single swarm child. Implementations must be infallible: dispatch
/// failures are reported as a `Failed` outcome, not as an `Err`, so the
/// scheduler can always return ordered results.
#[async_trait::async_trait]
pub trait AgentSwarmChildLauncher: Send + Sync {
    async fn launch(&self, spec: AgentSwarmChildSpec) -> AgentSwarmChildRun;
}

/// Observes a running swarm: `on_progress` is called once per child as it
/// resolves, with the running tally, so a client can render a live
/// `N/total done` tick. The final snapshot equals the aggregate result counts.
#[async_trait::async_trait]
pub trait AgentSwarmProgressObserver: Send + Sync {
    async fn on_progress(&self, snapshot: AgentSwarmProgressSnapshot);
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
    run_agent_swarm_with_observer(launcher, specs, config, cancel, None).await
}

/// Like [`run_agent_swarm`], but reports a live progress snapshot to `observer`
/// each time a child resolves.
pub async fn run_agent_swarm_with_observer(
    launcher: Arc<dyn AgentSwarmChildLauncher>,
    specs: Vec<AgentSwarmChildSpec>,
    config: &AgentSwarmConfig,
    cancel: AgentSwarmCancel,
    observer: Option<Arc<dyn AgentSwarmProgressObserver>>,
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

    let max_retries = config.rate_limit_max_retries;
    let base_backoff_ms = config.rate_limit_base_backoff_ms;
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
            // Launch the child, retrying with exponential backoff when the
            // provider rate-limits it. The concurrency permit is held across the
            // backoff so a rate-limited swarm naturally throttles instead of
            // hammering the provider. `biased` cancellation wins ties so an
            // in-flight or backing-off child is reported aborted (started)
            // rather than completing a turn the user already interrupted.
            let mut attempt = 0usize;
            loop {
                let run = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return (pos, ChildRunOrAbort::Started),
                    run = launcher.launch(spec.clone()) => run,
                };
                if !run.rate_limited || attempt >= max_retries {
                    return (pos, ChildRunOrAbort::Run(run));
                }
                // 3s, 6s, 12s, ... capped shift to avoid overflow.
                let backoff = Duration::from_millis(
                    base_backoff_ms.saturating_mul(1u64 << attempt.min(20)),
                );
                attempt += 1;
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return (pos, ChildRunOrAbort::Started),
                    _ = tokio::time::sleep(backoff) => {}
                }
            }
        });
    }

    let mut slots: Vec<Option<AgentSwarmChildResult>> = (0..total).map(|_| None).collect();
    let mut snapshot = AgentSwarmProgressSnapshot {
        total,
        ..AgentSwarmProgressSnapshot::default()
    };
    while let Some(joined) = set.join_next().await {
        if let Ok((pos, outcome)) = joined {
            let spec = &specs[pos];
            let child = match outcome {
                ChildRunOrAbort::Run(run) => child_result_from_run(spec, run),
                ChildRunOrAbort::Started => aborted_child(spec, AgentSwarmChildState::Started),
                ChildRunOrAbort::NotStarted => {
                    aborted_child(spec, AgentSwarmChildState::NotStarted)
                }
            };
            match child.outcome {
                AgentSwarmChildOutcome::Completed => snapshot.completed += 1,
                AgentSwarmChildOutcome::Failed => snapshot.failed += 1,
                AgentSwarmChildOutcome::Aborted => snapshot.aborted += 1,
            }
            slots[pos] = Some(child);
            if let Some(observer) = &observer {
                observer.on_progress(snapshot).await;
            }
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
                rate_limited: false,
            },
            Err(err) => {
                let message = err.to_string();
                let rate_limited = is_rate_limit_error(&message);
                AgentSwarmChildRun {
                    agent_id: None,
                    outcome: AgentSwarmChildOutcome::Failed,
                    body: message,
                    usage: None,
                    rate_limited,
                }
            }
        }
    }
}

/// Heuristic classifier for provider rate-limit failures, so the swarm
/// scheduler can requeue the child with backoff rather than failing it.
fn is_rate_limit_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("rate limit")
        || lower.contains("rate_limit")
        || lower.contains("ratelimit")
        || lower.contains("too many requests")
        || lower.contains("429")
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

        // When the runtime supplied a progress sink, publish a live tick as each
        // child resolves so clients can render "N/total done" mid-swarm.
        let observer: Option<Arc<dyn AgentSwarmProgressObserver>> =
            ctx.handles.swarm_progress_sink.clone().map(|sink| {
                Arc::new(SwarmProgressEmitter {
                    sink,
                    thread_id: ctx.thread_id.clone(),
                    turn_id: ctx.turn_id.clone(),
                    tool_id: call.id.clone(),
                }) as Arc<dyn AgentSwarmProgressObserver>
            });

        let result = run_agent_swarm_with_observer(
            launcher,
            specs,
            &self.config,
            AgentSwarmCancel::new(),
            observer,
        )
        .await;

        Ok(swarm_tool_result(call.id, call.name, result))
    }
}

/// Bridges the scheduler's [`AgentSwarmProgressObserver`] onto the runtime's
/// [`AgentSwarmProgressSink`], stamping each snapshot with the thread/turn/tool
/// ids so the runtime can emit a bus event without the scheduler knowing them.
struct SwarmProgressEmitter {
    sink: Arc<dyn AgentSwarmProgressSink>,
    thread_id: ThreadId,
    turn_id: TurnId,
    tool_id: String,
}

#[async_trait::async_trait]
impl AgentSwarmProgressObserver for SwarmProgressEmitter {
    async fn on_progress(&self, snapshot: AgentSwarmProgressSnapshot) {
        self.sink
            .emit_progress(&self.thread_id, &self.turn_id, &self.tool_id, snapshot)
            .await;
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
                    rate_limited: false,
                }
            } else {
                AgentSwarmChildRun {
                    agent_id: Some(format!("agent-{}", spec.index)),
                    outcome: AgentSwarmChildOutcome::Completed,
                    body: format!("child {} done", spec.index),
                    usage: None,
                    rate_limited: false,
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
            ..AgentSwarmConfig::default()
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

    #[derive(Default)]
    struct RecordingObserver {
        snapshots: Mutex<Vec<AgentSwarmProgressSnapshot>>,
    }

    #[async_trait::async_trait]
    impl AgentSwarmProgressObserver for RecordingObserver {
        async fn on_progress(&self, snapshot: AgentSwarmProgressSnapshot) {
            self.snapshots.lock().unwrap().push(snapshot);
        }
    }

    #[tokio::test]
    async fn observer_receives_incremental_progress_ending_at_final_counts() {
        let launcher = Arc::new(RecordingLauncher {
            fail_indices: vec![2],
            ..RecordingLauncher::default()
        });
        let specs: Vec<_> = (1..=3)
            .map(|i| spec(i, AgentSwarmChildKind::Spawn))
            .collect();
        let observer = Arc::new(RecordingObserver::default());
        // Cap concurrency at 1 so children resolve one-at-a-time, giving a
        // deterministic monotonically-increasing resolved count.
        let config = AgentSwarmConfig {
            max_concurrency: Some(1),
            ..fast_config()
        };
        let result = run_agent_swarm_with_observer(
            launcher,
            specs,
            &config,
            AgentSwarmCancel::new(),
            Some(observer.clone() as Arc<dyn AgentSwarmProgressObserver>),
        )
        .await;

        let snapshots = observer.snapshots.lock().unwrap().clone();
        // One tick per child, each carrying the total and a growing resolved count.
        assert_eq!(snapshots.len(), 3);
        let resolved: Vec<usize> = snapshots.iter().map(|s| s.resolved()).collect();
        assert_eq!(resolved, vec![1, 2, 3]);
        assert!(snapshots.iter().all(|s| s.total == 3));
        // Final tick equals the aggregate result.
        let last = snapshots.last().unwrap();
        assert_eq!(last.completed, result.completed);
        assert_eq!(last.failed, result.failed);
        assert_eq!(last.aborted, result.aborted);
        assert_eq!((last.completed, last.failed), (2, 1));
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
                rate_limited: false,
            }
        }
    }

    /// A launcher that rate-limits the first `rate_limit_attempts` launches of a
    /// child, then completes. Records the total attempt count so a test can
    /// prove the scheduler requeued rather than failing outright.
    struct RateLimitLauncher {
        attempts: Mutex<usize>,
        rate_limit_attempts: usize,
    }

    #[async_trait::async_trait]
    impl AgentSwarmChildLauncher for RateLimitLauncher {
        async fn launch(&self, spec: AgentSwarmChildSpec) -> AgentSwarmChildRun {
            let mut attempts = self.attempts.lock().unwrap();
            *attempts += 1;
            let this_attempt = *attempts;
            drop(attempts);
            if this_attempt <= self.rate_limit_attempts {
                AgentSwarmChildRun {
                    agent_id: Some(format!("agent-{}", spec.index)),
                    outcome: AgentSwarmChildOutcome::Failed,
                    body: "provider rate limit".to_string(),
                    usage: None,
                    rate_limited: true,
                }
            } else {
                AgentSwarmChildRun {
                    agent_id: Some(format!("agent-{}", spec.index)),
                    outcome: AgentSwarmChildOutcome::Completed,
                    body: "done after retry".to_string(),
                    usage: None,
                    rate_limited: false,
                }
            }
        }
    }

    #[tokio::test]
    async fn rate_limited_child_is_retried_with_backoff_then_completes() {
        // Two parallel children, each rate-limited twice then succeeding. Zero
        // base backoff keeps the test instant while still exercising the retry
        // loop deterministically.
        let launcher = Arc::new(RateLimitLauncher {
            attempts: Mutex::new(0),
            rate_limit_attempts: 4, // 2 children x 2 rate limits each
        });
        let specs = vec![
            spec(1, AgentSwarmChildKind::Spawn),
            spec(2, AgentSwarmChildKind::Spawn),
        ];
        let config = AgentSwarmConfig {
            rate_limit_max_retries: 4,
            rate_limit_base_backoff_ms: 0,
            ..fast_config()
        };
        let result = run_agent_swarm(launcher.clone(), specs, &config, AgentSwarmCancel::new()).await;
        assert_eq!(result.completed, 2, "both children recover after retries");
        assert_eq!(result.failed, 0);
        // 2 children x (2 rate-limited + 1 success) = 6 launches.
        assert_eq!(*launcher.attempts.lock().unwrap(), 6);
    }

    #[tokio::test]
    async fn rate_limited_child_fails_after_exhausting_retries() {
        // One child that always rate-limits; with max_retries = 2 it is launched
        // 3 times (initial + 2 retries) then reported failed (not aborted).
        let launcher = Arc::new(RateLimitLauncher {
            attempts: Mutex::new(0),
            rate_limit_attempts: 100,
        });
        let specs = vec![
            spec(1, AgentSwarmChildKind::Spawn),
            spec(2, AgentSwarmChildKind::Spawn),
        ];
        let config = AgentSwarmConfig {
            rate_limit_max_retries: 2,
            rate_limit_base_backoff_ms: 0,
            ..fast_config()
        };
        let result = run_agent_swarm(launcher.clone(), specs, &config, AgentSwarmCancel::new()).await;
        assert_eq!(result.failed, 2);
        assert_eq!(result.completed, 0);
        // 2 children x (1 + 2 retries) = 6 launches.
        assert_eq!(*launcher.attempts.lock().unwrap(), 6);
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
