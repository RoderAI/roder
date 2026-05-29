use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use roder_api::policy_mode::PolicyMode;
use roder_api::tools::{
    LocalWorkspaceHandle, ToolCall, ToolContributor, ToolExecutionContext, ToolRegistry,
};
use roder_ext_zerolang::{ZEROLANG_EDIT_TOOL, ZerolangConfig, ZerolangToolContributor};
use serde_json::{Value, json};

#[tokio::test]
async fn checked_graph_edit_builds_patch_runs_validations_and_returns_hunks() {
    let root = tempdir("success");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("src/main.0"),
        "pub fn main(world: World) -> Void raises {\n    check world.out.write(\"hello from zero\\n\")\n}\n",
    )
    .unwrap();
    let binary = fake_zero(&root, true);
    let result = execute_edit(
        &root,
        binary,
        json!({
            "input": "src/main.0",
            "graphHash": "graph:f76987e99677f1b3",
            "operations": [{
                "op": "set",
                "node": "#610c78bf",
                "field": "value",
                "expect": "hello from zero\n",
                "value": "hello from roder\n"
            }]
        }),
    )
    .await;

    assert!(!result["isError"].as_bool().unwrap());
    assert!(
        result["data"]["zerolang"]["patchText"]
            .as_str()
            .unwrap()
            .contains("expect graphHash \"graph:f76987e99677f1b3\"")
    );
    assert_eq!(
        result["data"]["zerolang"]["command"]["argv"],
        json!([
            "graph",
            "patch",
            "--json",
            "src/main.0",
            "--patch-text",
            result["data"]["zerolang"]["patchText"].as_str().unwrap()
        ])
    );
    assert_eq!(
        result["data"]["zerolang"]["validations"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(
        result["data"]["zerolang"]["hunks"]
            .as_array()
            .unwrap()
            .first()
            .unwrap()["afterLines"]
            .as_array()
            .unwrap()
            .iter()
            .any(|line| line.as_str().unwrap().contains("hello from roder"))
    );
}

#[tokio::test]
async fn checked_graph_edit_rejects_malformed_graph_hash_before_zero_launch() {
    let root = tempdir("bad-hash");
    let result = execute_edit(
        &root,
        PathBuf::from("/definitely/missing/zero"),
        json!({
            "input": "src/main.0",
            "graphHash": "f76987e99677f1b3",
            "operations": [{
                "op": "rename",
                "node": "#ea5ea1ca",
                "value": "start"
            }]
        }),
    )
    .await;

    assert!(result["isError"].as_bool().unwrap());
    assert!(
        result["text"]
            .as_str()
            .unwrap()
            .contains("graphHash must start with graph:")
    );
}

#[tokio::test]
async fn checked_graph_edit_surfaces_zero_patch_failure() {
    let root = tempdir("patch-failure");
    let binary = fake_zero(&root, false);
    let result = execute_edit(
        &root,
        binary,
        json!({
            "input": "src/main.0",
            "graphHash": "graph:f76987e99677f1b3",
            "operations": [{
                "op": "delete",
                "node": "#missing"
            }]
        }),
    )
    .await;

    assert!(result["isError"].as_bool().unwrap());
    assert!(
        result["text"]
            .as_str()
            .unwrap()
            .contains("zerolang graph patch failed")
    );
}

async fn execute_edit(root: &Path, binary: PathBuf, arguments: Value) -> Value {
    let mut registry = ToolRegistry::default();
    ZerolangToolContributor::new(ZerolangConfig {
        binary: Some(binary),
        timeout_seconds: Some(5),
        artifact_dir: None,
    })
    .contribute(&mut registry)
    .unwrap();
    let ctx = ToolExecutionContext::new("thread", "turn", PolicyMode::Default)
        .with_workspace_handle(Arc::new(LocalWorkspaceHandle::new(root)));
    let result = registry
        .get(ZEROLANG_EDIT_TOOL)
        .unwrap()
        .execute(
            ctx,
            ToolCall {
                id: "call-edit".to_string(),
                name: ZEROLANG_EDIT_TOOL.to_string(),
                arguments,
                raw_arguments: "{}".to_string(),
                thread_id: "thread".to_string(),
                turn_id: "turn".to_string(),
            },
        )
        .await
        .unwrap();
    json!({
        "text": result.text,
        "data": result.data,
        "isError": result.is_error
    })
}

fn fake_zero(root: &Path, patch_ok: bool) -> PathBuf {
    let path = root.join("zero");
    let patch_branch = if patch_ok {
        "perl -0pi -e 's/hello from zero/hello from roder/g' src/main.0\nprintf '{\"ok\":true,\"originalGraphHash\":\"graph:f76987e99677f1b3\",\"patchedGraphHash\":\"graph:aaaaaaaaaaaaaaaa\",\"saved\":{\"path\":\"src/main.0\"}}\\n'\n"
    } else {
        "printf 'stale node\\n' >&2\nprintf '{\"ok\":false,\"diagnostic\":{\"code\":\"GPH005\"}}\\n'\nexit 2\n"
    };
    fs::write(
        &path,
        format!(
            "#!/bin/sh\nif [ \"$1 $2 $3\" = \"graph patch --json\" ]; then\n{patch_branch}fi\nif [ \"$1 $2 $3\" = \"graph check --json\" ]; then\nprintf '{{\"ok\":true,\"graphHash\":\"graph:aaaaaaaaaaaaaaaa\"}}\\n'\nexit 0\nfi\nif [ \"$1 $2\" = \"check --json\" ]; then\nprintf '{{\"ok\":true}}\\n'\nexit 0\nfi\nprintf '{{\"ok\":true}}\\n'\n"
        ),
    )
    .unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).unwrap();
    path
}

fn tempdir(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "roder-zerolang-checked-{name}-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}
