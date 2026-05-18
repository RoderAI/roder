use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use roder_api::events::RoderEvent;
use roder_api::remote_runner::{RemoteRunnerProvider, RunnerDestination, RunnerManifest};
use roder_api::tasks::TaskState;
use roder_ext_runner_unix_local::UnixLocalRunnerProvider;
use roder_ext_task_process::ProcessTaskExecutor;
use roder_tasks::{
    BackgroundRunner, BackgroundRunnerConfig, TaskExecutorRegistry, TaskSubmitOptions,
};

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
