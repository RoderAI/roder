use std::path::PathBuf;
use std::sync::Arc;

use roder_api::policy_mode::PolicyMode;
use roder_api::tools::{
    LocalWorkspaceHandle, ToolCall, ToolContributor, ToolExecutionContext, ToolRegistry,
};
use roder_ext_webwright::{
    WEBWRIGHT_LIST_ARTIFACTS_TOOL, WEBWRIGHT_PREPARE_WORKSPACE_TOOL, WebwrightToolContributor,
};
use serde_json::json;

fn tempdir(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "roder-webwright-tools-it-{name}-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}

#[tokio::test]
async fn helper_tools_prepare_and_list_workspace() {
    let root = tempdir("prepare-list");
    let mut registry = ToolRegistry::default();
    WebwrightToolContributor.contribute(&mut registry).unwrap();
    let ctx = ToolExecutionContext::new("thread", "turn", PolicyMode::Default)
        .with_workspace_handle(Arc::new(LocalWorkspaceHandle::new(root.clone())));

    let result = registry
        .get(WEBWRIGHT_PREPARE_WORKSPACE_TOOL)
        .unwrap()
        .execute(
            ctx.clone(),
            ToolCall {
                id: "call-prepare".to_string(),
                name: WEBWRIGHT_PREPARE_WORKSPACE_TOOL.to_string(),
                arguments: json!({ "task": "Open a fixture page", "taskId": "fixture" }),
                raw_arguments: "{}".to_string(),
                thread_id: "thread".to_string(),
                turn_id: "turn".to_string(),
            },
        )
        .await
        .unwrap();
    assert!(!result.is_error);

    let listed = registry
        .get(WEBWRIGHT_LIST_ARTIFACTS_TOOL)
        .unwrap()
        .execute(
            ctx,
            ToolCall {
                id: "call-list".to_string(),
                name: WEBWRIGHT_LIST_ARTIFACTS_TOOL.to_string(),
                arguments: json!({ "workspace": ".roder/webwright/fixture" }),
                raw_arguments: "{}".to_string(),
                thread_id: "thread".to_string(),
                turn_id: "turn".to_string(),
            },
        )
        .await
        .unwrap();
    assert!(!listed.is_error);
    assert_eq!(
        listed.data["webwright"]["workspace"]["manifest"]["taskId"],
        "fixture"
    );
}
