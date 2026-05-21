use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use roder_api::events::RoderEvent;
use roder_api::processes::{ProcessOrigin, ProcessState};
use roder_api::remote_runner::{
    RemoteRunnerProvider, RemoteRunnerSession, RunnerArtifactExportRequest,
    RunnerArtifactExportResult, RunnerCommandId, RunnerCommandRequest, RunnerCommandResult,
    RunnerDestination, RunnerFileReadRequest, RunnerFileReadResult, RunnerFileWriteRequest,
    RunnerManifest, RunnerPortRequest, RunnerPortResult, RunnerSessionState, RunnerSnapshotRef,
};
use roder_api::tasks::TaskState;
use roder_ext_runner_unix_local::UnixLocalRunnerProvider;
use roder_ext_task_process::ProcessTaskExecutor;
use roder_tasks::{
    BackgroundRunner, BackgroundRunnerConfig, TaskExecutorRegistry, TaskSubmitOptions,
};
use tokio::sync::Notify;

fn temp_workspace() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("roder-process-task-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(dir.join("subdir")).unwrap();
    dir
}

fn runner(max_log_bytes: usize) -> BackgroundRunner {
    let mut registry = TaskExecutorRegistry::default();
    registry.register(Arc::new(ProcessTaskExecutor)).unwrap();
    BackgroundRunner::new(
        registry,
        BackgroundRunnerConfig {
            max_concurrent: 2,
            max_log_bytes,
            auto_cancel_on_session_end: true,
        },
    )
}

#[tokio::test]
async fn process_task_streams_stdout_and_stderr() {
    let workspace = temp_workspace();
    let runner = runner(1024);
    let mut events = runner.subscribe();
    let handle = runner
        .submit(
            "process",
            serde_json::json!({
                "command": "sh",
                "args": ["-c", "printf out; printf err >&2"],
                "cwd": ".",
            }),
            TaskSubmitOptions {
                workspace_root: Some(workspace.display().to_string()),
                ..TaskSubmitOptions::default()
            },
        )
        .await
        .unwrap();

    let mut output = String::new();
    let mut completed = None;
    while completed.is_none() {
        match tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap()
        {
            RoderEvent::TaskOutput(event) => output.push_str(&event.chunk),
            RoderEvent::TaskCompleted(event) => completed = Some(event),
            _ => {}
        }
    }

    assert!(output.contains("out"));
    assert!(output.contains("err"));
    assert_eq!(completed.unwrap().exit_code, Some(0));
    assert_eq!(
        runner.get(&handle.task_id).await.unwrap().state,
        TaskState::Completed
    );
}

#[tokio::test]
async fn process_task_honors_cwd_and_env_overrides() {
    let workspace = temp_workspace();
    let runner = runner(1024);
    let handle = runner
        .submit(
            "process",
            serde_json::json!({
                "command": "sh",
                "args": ["-c", "printf '%s:%s' \"$(basename \"$PWD\")\" \"$RODER_PROCESS_TEST\""],
                "cwd": "subdir",
                "env_overrides": { "RODER_PROCESS_TEST": "ok" },
            }),
            TaskSubmitOptions {
                workspace_root: Some(workspace.display().to_string()),
                ..TaskSubmitOptions::default()
            },
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let (logs, _) = runner.logs(&handle.task_id).await.unwrap();
    assert_eq!(
        logs.iter()
            .map(|entry| entry.chunk.as_str())
            .collect::<String>(),
        "subdir:ok"
    );
}

#[tokio::test]
async fn process_task_registers_process_descriptor_and_stops_from_registry() {
    let workspace = temp_workspace();
    let runner = runner(1024);
    let handle = runner
        .submit(
            "process",
            serde_json::json!({
                "command": "sh",
                "args": ["-c", "sleep 5"],
            }),
            TaskSubmitOptions {
                workspace_root: Some(workspace.display().to_string()),
                thread_id: Some("thread-process".to_string()),
                turn_id: Some("turn-process".to_string()),
                ..TaskSubmitOptions::default()
            },
        )
        .await
        .unwrap();
    let registry = runner.processes();

    for _ in 0..50 {
        if registry
            .list(false)
            .await
            .iter()
            .any(|process| process.task_id.as_deref() == Some(&handle.task_id))
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let process = registry
        .list(false)
        .await
        .into_iter()
        .find(|process| process.task_id.as_deref() == Some(&handle.task_id))
        .expect("process descriptor");
    assert_eq!(
        process.origin,
        roder_api::processes::ProcessOrigin::BackgroundTask
    );
    assert!(process.stoppable);
    assert_eq!(process.thread_id.as_deref(), Some("thread-process"));

    let stopped = registry
        .stop(&process.process_id, Some("test stop".to_string()))
        .await
        .unwrap();
    assert!(stopped.stopped);
    for _ in 0..50 {
        if let Some(process) = registry.get(&process.process_id).await
            && matches!(process.state, roder_api::processes::ProcessState::Stopped)
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("process did not stop");
}

#[tokio::test]
async fn process_task_routes_through_remote_runner_session_when_configured() {
    let workspace = temp_workspace();
    let runner = runner(1024);
    let provider = UnixLocalRunnerProvider::default();
    let destination = RunnerDestination {
        id: "unix-local".to_string(),
        provider_id: "unix-local".to_string(),
        config: serde_json::json!({ "root": workspace.display().to_string() }),
        default_manifest: RunnerManifest::default(),
    };
    let session = provider.create_session(destination.clone()).await.unwrap();
    let handle = runner
        .submit(
            "process",
            serde_json::json!({
                "command": "sh",
                "args": ["-c", "printf '%s:%s' \"$(basename \"$PWD\")\" \"$RODER_PROCESS_TEST\""],
                "cwd": "subdir",
                "env_overrides": { "RODER_PROCESS_TEST": "remote-ok" },
            }),
            TaskSubmitOptions {
                workspace_root: Some(workspace.display().to_string()),
                runner_destination: Some(destination),
                runner_session: Some(session),
                ..TaskSubmitOptions::default()
            },
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let (logs, _) = runner.logs(&handle.task_id).await.unwrap();
    assert_eq!(
        logs.iter()
            .map(|entry| entry.chunk.as_str())
            .collect::<String>(),
        "subdir:remote-ok"
    );
}

#[tokio::test]
async fn remote_process_descriptor_uses_runner_ids_and_provider_cancel() {
    let workspace = temp_workspace();
    let runner = runner(1024);
    let session = Arc::new(FakeRemoteSession::new("fake-destination", true));
    let destination = RunnerDestination {
        id: "fake-destination".to_string(),
        provider_id: "fake".to_string(),
        config: serde_json::Value::Null,
        default_manifest: RunnerManifest::default(),
    };
    let handle = runner
        .submit(
            "process",
            serde_json::json!({
                "command": "remote-program",
                "args": ["--long"],
            }),
            TaskSubmitOptions {
                workspace_root: Some(workspace.display().to_string()),
                runner_destination: Some(destination),
                runner_session: Some(session.clone()),
                ..TaskSubmitOptions::default()
            },
        )
        .await
        .unwrap();
    let registry = runner.processes();

    let process = wait_for_task_process(&registry, &handle.task_id).await;
    assert_eq!(process.origin, ProcessOrigin::RemoteRunner);
    assert_eq!(process.pid, None);
    assert_eq!(
        process.runner_destination_id.as_deref(),
        Some("fake-destination")
    );
    assert_eq!(process.runner_session_id.as_deref(), Some("fake-session"));

    let stopped = registry
        .stop(&process.process_id, Some("test remote cancel".to_string()))
        .await
        .unwrap();
    assert!(stopped.stopped);
    assert!(session.cancel_called.load(Ordering::SeqCst));

    for _ in 0..50 {
        if let Some(process) = registry.get(&process.process_id).await
            && matches!(process.state, ProcessState::Exited { exit_code: None })
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("remote process did not record provider completion");
}

#[tokio::test]
async fn remote_process_cancel_failure_leaves_process_running() {
    let workspace = temp_workspace();
    let runner = runner(1024);
    let session = Arc::new(FakeRemoteSession::new("fake-destination", false));
    let destination = RunnerDestination {
        id: "fake-destination".to_string(),
        provider_id: "fake".to_string(),
        config: serde_json::Value::Null,
        default_manifest: RunnerManifest::default(),
    };
    let handle = runner
        .submit(
            "process",
            serde_json::json!({
                "command": "remote-program",
                "args": ["--long"],
            }),
            TaskSubmitOptions {
                workspace_root: Some(workspace.display().to_string()),
                runner_destination: Some(destination),
                runner_session: Some(session.clone()),
                ..TaskSubmitOptions::default()
            },
        )
        .await
        .unwrap();
    let registry = runner.processes();
    let process = wait_for_task_process(&registry, &handle.task_id).await;

    let err = registry
        .stop(&process.process_id, Some("test remote cancel".to_string()))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("did not cancel"));
    assert!(session.cancel_called.load(Ordering::SeqCst));
    let process = registry.get(&process.process_id).await.unwrap();
    assert!(matches!(process.state, ProcessState::Running));

    runner
        .cancel(&handle.task_id, Some("cleanup".to_string()))
        .await
        .unwrap();
}

#[tokio::test]
async fn process_task_can_cancel_remote_runner_command_handle() {
    let workspace = temp_workspace();
    let runner = runner(1024);
    let provider = UnixLocalRunnerProvider::default();
    let destination = RunnerDestination {
        id: "unix-local".to_string(),
        provider_id: "unix-local".to_string(),
        config: serde_json::json!({ "root": workspace.display().to_string() }),
        default_manifest: RunnerManifest::default(),
    };
    let session = provider.create_session(destination.clone()).await.unwrap();
    let handle = runner
        .submit(
            "process",
            serde_json::json!({
                "command": "sh",
                "args": ["-c", "sleep 5"],
            }),
            TaskSubmitOptions {
                workspace_root: Some(workspace.display().to_string()),
                runner_destination: Some(destination),
                runner_session: Some(session),
                ..TaskSubmitOptions::default()
            },
        )
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(25)).await;
    assert!(
        runner
            .cancel(&handle.task_id, Some("test cancel".to_string()))
            .await
            .unwrap()
    );
    assert_eq!(
        runner.get(&handle.task_id).await.unwrap().state,
        TaskState::Cancelled
    );
}

#[tokio::test]
async fn process_task_reports_non_zero_exit_as_completed_event() {
    let workspace = temp_workspace();
    let runner = runner(1024);
    let mut events = runner.subscribe();
    runner
        .submit(
            "process",
            serde_json::json!({
                "command": "sh",
                "args": ["-c", "exit 7"],
            }),
            TaskSubmitOptions {
                workspace_root: Some(workspace.display().to_string()),
                ..TaskSubmitOptions::default()
            },
        )
        .await
        .unwrap();

    loop {
        if let RoderEvent::TaskCompleted(event) =
            tokio::time::timeout(Duration::from_secs(2), events.recv())
                .await
                .unwrap()
                .unwrap()
        {
            assert_eq!(event.exit_code, Some(7));
            assert_eq!(event.payload["success"], false);
            break;
        }
    }
}

#[tokio::test]
async fn process_task_buffers_slow_output_with_drops() {
    let workspace = temp_workspace();
    let runner = runner(8);
    let handle = runner
        .submit(
            "process",
            serde_json::json!({
                "command": "sh",
                "args": ["-c", "printf 12345; sleep 0.02; printf 67890"],
            }),
            TaskSubmitOptions {
                workspace_root: Some(workspace.display().to_string()),
                ..TaskSubmitOptions::default()
            },
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;

    let (logs, dropped) = runner.logs(&handle.task_id).await.unwrap();
    assert_eq!(
        logs.iter()
            .map(|entry| entry.chunk.as_str())
            .collect::<String>(),
        "34567890"
    );
    assert!(dropped > 0);
}

async fn wait_for_task_process(
    registry: &roder_tasks::ProcessRegistry,
    task_id: &str,
) -> roder_api::processes::ProcessDescriptor {
    for _ in 0..50 {
        if let Some(process) = registry
            .list(false)
            .await
            .into_iter()
            .find(|process| process.task_id.as_deref() == Some(task_id))
        {
            return process;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("process descriptor for task {task_id} not found");
}

struct FakeRemoteSession {
    state: RunnerSessionState,
    cancel_succeeds: bool,
    cancel_called: AtomicBool,
    cancelled: Notify,
}

impl FakeRemoteSession {
    fn new(destination_id: &str, cancel_succeeds: bool) -> Self {
        Self {
            state: RunnerSessionState {
                provider_id: "fake".to_string(),
                session_id: "fake-session".to_string(),
                destination_id: destination_id.to_string(),
                snapshot: None,
                metadata: serde_json::Value::Null,
            },
            cancel_succeeds,
            cancel_called: AtomicBool::new(false),
            cancelled: Notify::new(),
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
        self.cancelled.notified().await;
        Ok(RunnerCommandResult {
            command_id: request.command_id,
            exit_code: None,
            stdout: "remote cancelled\n".to_string(),
            stderr: String::new(),
        })
    }

    async fn cancel_command(&self, _command_id: &RunnerCommandId) -> anyhow::Result<bool> {
        self.cancel_called.store(true, Ordering::SeqCst);
        if self.cancel_succeeds {
            self.cancelled.notify_waiters();
            Ok(true)
        } else {
            Ok(false)
        }
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
