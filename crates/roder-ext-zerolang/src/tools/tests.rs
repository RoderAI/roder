use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use roder_api::policy_mode::PolicyMode;
use roder_api::tools::{LocalWorkspaceHandle, ToolCall, ToolContributor, ToolExecutionContext};
use serde_json::{Value, json};

use super::*;

fn call(name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: format!("call-{name}"),
        name: name.to_string(),
        arguments,
        raw_arguments: "{}".to_string(),
        thread_id: "thread".to_string(),
        turn_id: "turn".to_string(),
    }
}

fn tempdir(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "roder-zerolang-tools-{name}-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn fake_zero(root: &Path) -> PathBuf {
    #[cfg(unix)]
    {
        let path = root.join("zero");
        fs::write(
            &path,
            "#!/bin/sh\nprintf '{\"ok\":true,\"argv\":\"%s\"}\\n' \"$*\"\n",
        )
        .unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).unwrap();
        path
    }

    #[cfg(windows)]
    {
        let path = root.join("zero.cmd");
        fs::write(&path, "@echo off\r\necho {\"ok\":true,\"argv\":\"%*\"}\r\n").unwrap();
        path
    }
}

#[tokio::test]
async fn contributor_registers_exact_zerolang_tool_names() {
    let mut registry = ToolRegistry::default();
    ZerolangToolContributor::default()
        .contribute(&mut registry)
        .unwrap();
    let names = registry
        .specs()
        .into_iter()
        .map(|spec| spec.name)
        .collect::<Vec<_>>();

    for expected in [
        ZEROLANG_SKILLS_GET_TOOL,
        ZEROLANG_CHECK_TOOL,
        ZEROLANG_GRAPH_DUMP_TOOL,
        ZEROLANG_GRAPH_VIEW_TOOL,
        ZEROLANG_FIX_PLAN_TOOL,
        ZEROLANG_EDIT_TOOL,
        ZEROLANG_GRAPH_ROUNDTRIP_TOOL,
    ] {
        assert!(names.contains(&expected.to_string()), "missing {expected}");
    }
    assert!(!names.contains(&"zeolang_edit".to_string()));
}

#[tokio::test]
async fn normalized_schema_keeps_zerolang_edit_required_fields() {
    let mut registry = ToolRegistry::default();
    ZerolangToolContributor::default()
        .contribute(&mut registry)
        .unwrap();

    let specs = registry.specs_for_edit_tool(Some("patch"));
    let edit = specs
        .into_iter()
        .find(|spec| spec.name == ZEROLANG_EDIT_TOOL)
        .unwrap();

    assert_eq!(
        edit.parameters["required"],
        json!(["input", "graphHash", "operations"])
    );
    assert_eq!(
        edit.parameters["properties"]["operations"]["items"]["required"],
        json!(["op"])
    );
    assert_eq!(
        edit.parameters["properties"]["operations"]["items"]["additionalProperties"],
        false
    );
    assert!(
        edit.description.contains("structured operation objects"),
        "{}",
        edit.description
    );
    assert!(
        edit.description
            .contains("Do not pass zero_graph_patch-style args"),
        "{}",
        edit.description
    );
    assert_eq!(
        edit.parameters["properties"]["graphHash"]["pattern"],
        json!("^graph:[0-9a-f]{16}$")
    );
    assert!(
        edit.parameters["properties"]["operations"]["description"]
            .as_str()
            .unwrap()
            .contains("structured Roder operation")
    );
    let operation_properties = &edit.parameters["properties"]["operations"]["items"]["properties"];
    assert!(operation_properties.get("id").is_none());
    assert!(
        operation_properties["node"]["description"]
            .as_str()
            .unwrap()
            .contains("Do not use `id`")
    );
    assert!(
        operation_properties["value"]["description"]
            .as_str()
            .unwrap()
            .contains("string")
    );
}

#[tokio::test]
async fn check_tool_uses_configured_zero_binary_without_shell() {
    let root = tempdir("check");
    let binary = fake_zero(&root);
    let ctx = ToolExecutionContext::new("thread", "turn", PolicyMode::Default)
        .with_workspace_handle(Arc::new(LocalWorkspaceHandle::new(root.clone())));
    let tool = CheckTool::new(ZerolangConfig {
        binary: Some(binary),
        timeout_seconds: Some(5),
        artifact_dir: None,
    });

    let result = tool
        .execute(ctx, call(ZEROLANG_CHECK_TOOL, json!({ "input": "main.0" })))
        .await
        .unwrap();

    assert!(!result.is_error);
    assert_eq!(
        result.data["zerolang"]["command"]["argv"],
        json!(["check", "--json", "main.0"])
    );
}

#[tokio::test]
async fn edit_tool_returns_patch_text_before_launch_errors() {
    let root = tempdir("edit-error");
    let ctx = ToolExecutionContext::new("thread", "turn", PolicyMode::Default)
        .with_workspace_handle(Arc::new(LocalWorkspaceHandle::new(root)));
    let tool = EditTool::new(ZerolangConfig {
        binary: Some(PathBuf::from("/missing/zero")),
        timeout_seconds: Some(1),
        artifact_dir: None,
    });

    let result = tool
        .execute(
            ctx,
            call(
                ZEROLANG_EDIT_TOOL,
                json!({
                    "input": "main.0",
                    "graphHash": "graph:f76987e99677f1b3",
                    "operations": [{
                        "op": "rename",
                        "node": "#ea5ea1ca",
                        "expect": "main",
                        "value": "start"
                    }]
                }),
            ),
        )
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(
        result.data["zerolang"]["patchText"]
            .as_str()
            .unwrap()
            .contains("rename node=\"#ea5ea1ca\" expect=\"main\" value=\"start\"")
    );
}
