use std::path::PathBuf;

use roder_api::tasks::{TaskExecutionContext, TaskExecutor, TaskOutputSink};
use roder_ext_webwright::{WEBWRIGHT_TASK_EXECUTOR_ID, WebwrightTaskExecutor};
use serde_json::json;

fn tempdir(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "roder-webwright-task-it-{name}-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}

#[tokio::test]
async fn task_executor_prepares_workspace_payload() {
    let root = tempdir("prepare");
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
                "task": "Open a fixture page",
                "mode": "run",
                "taskId": "fixture"
            }),
        )
        .await
        .unwrap();

    assert_eq!(executor.id(), WEBWRIGHT_TASK_EXECUTOR_ID);
    assert_eq!(result.payload["webwright"]["taskId"], "fixture");
    assert!(
        root.join(".roder/webwright/fixture/final_script.py")
            .exists()
    );
}
