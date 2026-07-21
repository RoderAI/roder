use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use roder_api::dynamic_workflows::{
    WorkflowAgentStatus, WorkflowRunLimits, WorkflowRunStatus, WorkflowScript,
    WorkflowScriptSource, WorkflowScriptSourceKind,
};
use roder_api::subagents::{SubagentExitReason, SubagentResult};
use roder_api::tasks::{TaskExecutionContext, TaskExecutor, TaskOutputSink};
use roder_dynamic_workflows::{
    WorkflowAgentExecutionContext, WorkflowAgentExecutionRequest, WorkflowAgentExecutor,
    WorkflowCheckpointStore, WorkflowRunRequest, WorkflowRunner, WorkflowTaskExecutor,
    workflow_script_hash,
};
use time::OffsetDateTime;
use tokio::sync::Mutex;

const WORKFLOW: &str = r#"
workflow.define({
  name: "Two Phase Audit",
  hostApiVersion: 1,
  phases: ["Scout", "Review"],
  limits: { maxConcurrentAgents: 2, maxAgentsPerRun: 8 }
}, async (ctx) => {
  ctx.phase.start("Scout");
  ctx.agents.map("scout", ["api", "core", "tui"], (target) => ({
    lane: "scout",
    description: `inspect ${target}`,
    prompt: `inspect ${target}`
  }));
  ctx.phase.start("Review");
  ctx.agents.run("reviewer", {
    lane: "reviewer",
    description: "review findings",
    prompt: "review findings"
  });
  return ctx.report.markdown("script report");
});
"#;

#[derive(Default)]
struct FakeWorkflowExecutor {
    active: AtomicUsize,
    peak: AtomicUsize,
    calls: Mutex<Vec<String>>,
    fail_once: Mutex<HashSet<String>>,
    delay: Duration,
}

#[async_trait]
impl WorkflowAgentExecutor for FakeWorkflowExecutor {
    async fn execute_agent(
        &self,
        context: WorkflowAgentExecutionContext,
        request: WorkflowAgentExecutionRequest,
    ) -> anyhow::Result<SubagentResult> {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(active, Ordering::SeqCst);
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        self.active.fetch_sub(1, Ordering::SeqCst);

        let prompt = request.subagent_request.prompt.clone();
        self.calls.lock().await.push(prompt.clone());
        if context.stopped.load(Ordering::SeqCst) {
            return Ok(result_for(
                context.agent_id,
                request.launch.role,
                prompt,
                SubagentExitReason::Cancelled,
            ));
        }
        if self.fail_once.lock().await.remove(&prompt) {
            return Ok(result_for(
                context.agent_id,
                request.launch.role,
                prompt,
                SubagentExitReason::Failed,
            ));
        }
        Ok(result_for(
            context.agent_id,
            request.launch.role,
            prompt,
            SubagentExitReason::Completed,
        ))
    }
}

#[tokio::test]
async fn runner_executes_two_phase_fanout_with_bounded_concurrency_and_report() {
    let executor = Arc::new(FakeWorkflowExecutor {
        delay: Duration::from_millis(20),
        ..Default::default()
    });
    let runner = runner(executor.clone());
    let handle = runner
        .start(request("run-fanout", false))
        .await
        .expect("workflow starts");

    let snapshot = wait_with_timeout(&handle).await;

    assert_eq!(snapshot.run.status, WorkflowRunStatus::Completed);
    assert_eq!(snapshot.run.phases.len(), 2);
    assert_eq!(snapshot.run.agents.len(), 4);
    assert_eq!(
        snapshot.run.summary.as_ref().unwrap().completed_agent_count,
        4
    );
    assert_eq!(snapshot.run.agents[0].model.as_deref(), Some("fake-model"));
    assert!(executor.peak.load(Ordering::SeqCst) <= 2);
    assert!(
        snapshot
            .report
            .as_deref()
            .unwrap()
            .contains("actual:inspect api")
    );
}

#[tokio::test]
async fn pause_before_launch_waits_for_resume_and_completed_results_are_reused() {
    let executor = Arc::new(FakeWorkflowExecutor::default());
    let runner = runner(executor.clone());
    let handle = runner
        .start(request("run-reuse", true))
        .await
        .expect("workflow starts paused");

    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(executor.calls.lock().await.is_empty());
    assert_eq!(
        handle.snapshot().await.run.status,
        WorkflowRunStatus::Paused
    );

    handle.resume().await;
    let first = wait_with_timeout(&handle).await;
    assert_eq!(first.run.status, WorkflowRunStatus::Completed);
    assert_eq!(executor.calls.lock().await.len(), 4);

    let second_handle = runner
        .start(request("run-reuse", false))
        .await
        .expect("workflow restarts same run id");
    let second = wait_with_timeout(&second_handle).await;
    assert_eq!(second.reused_agent_results, 4);
    assert_eq!(executor.calls.lock().await.len(), 4);
}

#[tokio::test]
async fn stop_while_running_marks_run_stopped() {
    let executor = Arc::new(FakeWorkflowExecutor {
        delay: Duration::from_millis(80),
        ..Default::default()
    });
    let runner = runner(executor);
    let handle = runner
        .start(request("run-stop", false))
        .await
        .expect("workflow starts");

    tokio::time::sleep(Duration::from_millis(10)).await;
    handle.stop(Some("test stop".to_string())).await;
    let snapshot = wait_with_timeout(&handle).await;

    assert_eq!(snapshot.run.status, WorkflowRunStatus::Stopped);
    assert!(
        snapshot
            .run
            .agents
            .iter()
            .any(|agent| agent.status == WorkflowAgentStatus::Cancelled)
    );
}

#[tokio::test]
async fn restart_agent_reruns_failed_child() {
    let executor = Arc::new(FakeWorkflowExecutor::default());
    executor
        .fail_once
        .lock()
        .await
        .insert("inspect api".to_string());
    let runner = runner(executor.clone());
    let handle = runner
        .start(request("run-restart", false))
        .await
        .expect("workflow starts");
    let first = wait_with_timeout(&handle).await;

    assert_eq!(first.run.agents[0].status, WorkflowAgentStatus::Failed);
    assert!(handle.restart_agent("agent-1").await.unwrap());

    let restarted = handle.snapshot().await;
    assert_eq!(
        restarted.run.agents[0].status,
        WorkflowAgentStatus::Completed
    );
    assert_eq!(executor.calls.lock().await[0], "inspect api");
    assert_eq!(executor.calls.lock().await.last().unwrap(), "inspect api");
}

#[tokio::test]
async fn task_executor_runs_workflow_through_background_task_contract() {
    let executor = Arc::new(FakeWorkflowExecutor::default());
    let task = WorkflowTaskExecutor::new(runner(executor));
    let result = task
        .execute(
            TaskExecutionContext {
                task_id: "task-1".to_string(),
                thread_id: Some("thread-from-task".to_string()),
                turn_id: Some("turn-from-task".to_string()),
                workspace_root: None,
                runner_destination: None,
                runner_session: None,
                deadline: None,
                process_grace_timeout: std::time::Duration::from_millis(250),
                process_kill_timeout: std::time::Duration::from_secs(1),
                metadata: serde_json::json!({}),
                process_registry: None,
                output: TaskOutputSink::default(),
            },
            serde_json::to_value(request("run-task", false)).unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(result.payload["run"]["status"], "completed");
    assert_eq!(result.payload["run"]["threadId"], "thread");
    assert!(
        result.payload["report"]
            .as_str()
            .unwrap()
            .contains("actual:")
    );
}

fn runner(executor: Arc<dyn WorkflowAgentExecutor>) -> WorkflowRunner {
    WorkflowRunner::new(
        executor,
        WorkflowCheckpointStore::new(temp_root()),
        Default::default(),
    )
}

fn request(run_id: &str, start_paused: bool) -> WorkflowRunRequest {
    WorkflowRunRequest {
        run_id: run_id.to_string(),
        thread_id: Some("thread".to_string()),
        turn_id: Some("turn".to_string()),
        script: script(),
        arguments: serde_json::json!({}),
        start_paused,
        approval: None,
    }
}

fn script() -> WorkflowScript {
    let now = OffsetDateTime::UNIX_EPOCH;
    WorkflowScript {
        script_id: "script-two-phase".to_string(),
        name: "Two Phase Audit".to_string(),
        description: None,
        source: WorkflowScriptSource {
            kind: WorkflowScriptSourceKind::Generated,
            path: None,
            command_name: None,
            extension_id: None,
        },
        hash: workflow_script_hash(WORKFLOW),
        host_api_version: 1,
        arguments_schema: serde_json::json!({}),
        body: Some(WORKFLOW.to_string()),
        limits: WorkflowRunLimits::default(),
        created_at: now,
        updated_at: now,
    }
}

fn result_for(
    agent_id: String,
    role: String,
    prompt: String,
    exit_reason: SubagentExitReason,
) -> SubagentResult {
    SubagentResult {
        thread_id: format!("thread-{agent_id}"),
        turn_id: format!("turn-{agent_id}"),
        agent_type: role,
        model: Some("fake-model".to_string()),
        final_message: format!("actual:{prompt}"),
        usage: None,
        exit_reason,
        transcript: None,
        metadata: serde_json::json!({}),
    }
}

fn temp_root() -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("roder-workflow-runner-{nanos}"))
}

async fn wait_with_timeout(
    handle: &roder_dynamic_workflows::WorkflowRunHandle,
) -> roder_dynamic_workflows::WorkflowRunSnapshot {
    match tokio::time::timeout(Duration::from_secs(2), handle.wait()).await {
        Ok(result) => result.expect("workflow finishes"),
        Err(_) => panic!("workflow timed out: {:?}", handle.snapshot().await),
    }
}
