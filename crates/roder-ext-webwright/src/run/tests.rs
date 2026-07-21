use std::sync::{Arc, Mutex};

use roder_api::processes::{ProcessDescriptor, ProcessOutput, ProcessRegistrySink, ProcessStopper};
use roder_api::remote_runner::{
    RemoteRunnerSession, RunnerArtifactExportRequest, RunnerArtifactExportResult,
    RunnerCommandRequest, RunnerCommandResult, RunnerFileReadRequest, RunnerFileReadResult,
    RunnerFileWriteRequest, RunnerPortRequest, RunnerPortResult, RunnerSessionState,
    RunnerSnapshotRef,
};
use roder_api::tasks::{TaskExecutionContext, TaskExecutor, TaskOutputSink, TaskState};
use serde_json::json;

use super::{WEBWRIGHT_TASK_EXECUTOR_ID, WebwrightTaskExecutor, workspace_root};
use roder_timing_test::tempdir;

mod roder_timing_test {
    pub fn tempdir(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "roder-webwright-{name}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }
}

#[tokio::test]
async fn task_executor_prepares_workspace_without_dependency_check() {
    let root = tempdir("task_executor_prepares_workspace");
    let executor = WebwrightTaskExecutor::without_dependency_check();
    let ctx = TaskExecutionContext {
        task_id: "task-1".to_string(),
        thread_id: None,
        turn_id: None,
        workspace_root: Some(root.display().to_string()),
        runner_destination: None,
        runner_session: None,
        deadline: None,
        process_grace_timeout: std::time::Duration::from_millis(250),
        process_kill_timeout: std::time::Duration::from_secs(1),
        metadata: serde_json::Value::Null,
        process_registry: None,
        output: TaskOutputSink::default(),
    };
    let result = executor
        .execute(
            ctx,
            json!({
                "task": "Open fixture page and extract the heading",
                "mode": "run",
                "taskId": "fixture-heading"
            }),
        )
        .await
        .unwrap();

    assert_eq!(result.exit_code, None);
    assert_eq!(result.payload["webwright"]["taskId"], "fixture-heading");
    assert!(
        root.join(".roder/webwright/fixture-heading/plan.md")
            .exists()
    );
}

#[tokio::test]
async fn task_executor_records_remote_runner_preflight_process() {
    let root = tempdir("task_executor_remote_preflight");
    let registry = Arc::new(RecordingProcessRegistry::default());
    let session = Arc::new(FakeRemoteSession::new());
    let executor = WebwrightTaskExecutor::without_dependency_check();
    let ctx = TaskExecutionContext {
        task_id: "task-remote".to_string(),
        thread_id: None,
        turn_id: None,
        workspace_root: Some(root.display().to_string()),
        runner_destination: None,
        runner_session: Some(session.clone()),
        deadline: None,
        process_grace_timeout: std::time::Duration::from_millis(250),
        process_kill_timeout: std::time::Duration::from_secs(1),
        metadata: serde_json::Value::Null,
        process_registry: Some(registry.clone()),
        output: TaskOutputSink::default(),
    };

    executor
        .execute(
            ctx,
            json!({
                "task": "Open fixture page remotely",
                "mode": "run",
                "taskId": "remote-fixture"
            }),
        )
        .await
        .unwrap();

    let request = session.requests.lock().unwrap().first().cloned().unwrap();
    assert_eq!(request.program, "true");
    assert_eq!(
        request.cwd.as_deref(),
        Some(root.join(".roder/webwright/remote-fixture").as_path())
    );
    let process = registry.processes.lock().unwrap().first().cloned().unwrap();
    assert_eq!(process.task_id.as_deref(), Some("task-remote"));
    assert_eq!(process.runner_destination_id.as_deref(), Some("remote"));
    assert_eq!(process.runner_session_id.as_deref(), Some("session-1"));
}

#[test]
fn task_spec_names_webwright_executor() {
    let spec = WebwrightTaskExecutor::new().spec();
    assert_eq!(spec.kind, WEBWRIGHT_TASK_EXECUTOR_ID);
    assert_eq!(spec.default_timeout_seconds, Some(900));
    assert_ne!(TaskState::Queued, TaskState::Completed);
}

#[test]
fn workspace_root_rejects_output_dir_escapes() {
    let root = tempdir("workspace_root_rejects_escapes");
    let ctx = TaskExecutionContext {
        task_id: "task-1".to_string(),
        thread_id: None,
        turn_id: None,
        workspace_root: Some(root.display().to_string()),
        runner_destination: None,
        runner_session: None,
        deadline: None,
        process_grace_timeout: std::time::Duration::from_millis(250),
        process_kill_timeout: std::time::Duration::from_secs(1),
        metadata: serde_json::Value::Null,
        process_registry: None,
        output: TaskOutputSink::default(),
    };

    assert!(workspace_root(&ctx, Some(".roder/webwright/task"), "task").is_ok());
    assert!(workspace_root(&ctx, Some("../outside"), "task").is_err());
    assert!(
        workspace_root(
            &ctx,
            Some(&root.join("../outside").display().to_string()),
            "task"
        )
        .is_err()
    );
}

#[derive(Default)]
struct RecordingProcessRegistry {
    processes: Mutex<Vec<ProcessDescriptor>>,
}

#[async_trait::async_trait]
impl ProcessRegistrySink for RecordingProcessRegistry {
    async fn register_process(
        &self,
        process: ProcessDescriptor,
        _stopper: Option<Arc<dyn ProcessStopper>>,
    ) -> anyhow::Result<ProcessDescriptor> {
        self.processes.lock().unwrap().push(process.clone());
        Ok(process)
    }

    async fn append_process_output(&self, _output: ProcessOutput) -> anyhow::Result<()> {
        Ok(())
    }

    async fn mark_process_exited(
        &self,
        _process_id: &str,
        _exit_code: Option<i32>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn mark_process_failed(&self, _process_id: &str, _error: String) -> anyhow::Result<()> {
        Ok(())
    }

    async fn mark_process_stopped(
        &self,
        _process_id: &str,
        _reason: Option<String>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

struct FakeRemoteSession {
    state: RunnerSessionState,
    requests: Mutex<Vec<RunnerCommandRequest>>,
}

impl FakeRemoteSession {
    fn new() -> Self {
        Self {
            state: RunnerSessionState {
                provider_id: "fake".to_string(),
                session_id: "session-1".to_string(),
                destination_id: "remote".to_string(),
                snapshot: None,
                metadata: serde_json::Value::Null,
            },
            requests: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait::async_trait]
impl RemoteRunnerSession for FakeRemoteSession {
    fn state(&self) -> RunnerSessionState {
        self.state.clone()
    }

    async fn run_command(
        &self,
        request: RunnerCommandRequest,
    ) -> anyhow::Result<RunnerCommandResult> {
        self.requests.lock().unwrap().push(request.clone());
        Ok(RunnerCommandResult {
            command_id: request.command_id,
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
        })
    }

    async fn read_file(
        &self,
        _request: RunnerFileReadRequest,
    ) -> anyhow::Result<RunnerFileReadResult> {
        anyhow::bail!("not implemented")
    }

    async fn write_file(&self, _request: RunnerFileWriteRequest) -> anyhow::Result<()> {
        anyhow::bail!("not implemented")
    }

    async fn expose_port(&self, _request: RunnerPortRequest) -> anyhow::Result<RunnerPortResult> {
        anyhow::bail!("not implemented")
    }

    async fn export_artifact(
        &self,
        _request: RunnerArtifactExportRequest,
    ) -> anyhow::Result<RunnerArtifactExportResult> {
        anyhow::bail!("not implemented")
    }

    async fn snapshot(&self) -> anyhow::Result<Option<RunnerSnapshotRef>> {
        Ok(None)
    }

    async fn close(&self) -> anyhow::Result<()> {
        Ok(())
    }
}
