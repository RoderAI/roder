use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use roder_api::policy_mode::PolicyMode;
use roder_api::tools::{
    LocalProcessRunnerHandle, LocalWorkspaceHandle, ToolCall, ToolContributor,
    ToolExecutionContext, ToolRegistry,
};
use serde_json::json;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use crate::tools::{
    WEBWRIGHT_ALLOCATE_RUN_TOOL, WEBWRIGHT_LINT_SCRIPT_TOOL, WEBWRIGHT_LIST_ARTIFACTS_TOOL,
    WEBWRIGHT_PREPARE_WORKSPACE_TOOL, WEBWRIGHT_READ_LOG_TAIL_TOOL, WEBWRIGHT_RUN_SCRIPT_TOOL,
    WEBWRIGHT_SUMMARIZE_VERIFICATION_TOOL, WEBWRIGHT_VERIFY_RUN_TOOL, WebwrightToolContributor,
};

fn tempdir(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "roder-webwright-tools-{name}-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn call(name: &str, arguments: serde_json::Value) -> ToolCall {
    ToolCall {
        id: format!("call-{name}"),
        name: name.to_string(),
        arguments,
        raw_arguments: "{}".to_string(),
        thread_id: "thread".to_string(),
        turn_id: "turn".to_string(),
    }
}

fn webwright_context(root: PathBuf) -> ToolExecutionContext {
    ToolExecutionContext::new("thread", "turn", PolicyMode::Default)
        .with_workspace_handle(Arc::new(LocalWorkspaceHandle::new(root)))
        .with_process_runner(Arc::new(LocalProcessRunnerHandle))
}

fn webwright_context_without_process_runner(root: PathBuf) -> ToolExecutionContext {
    ToolExecutionContext::new("thread", "turn", PolicyMode::Default)
        .with_workspace_handle(Arc::new(LocalWorkspaceHandle::new(root)))
}

#[tokio::test]
async fn helper_tools_prepare_list_and_verify_workspace() {
    let root = tempdir("prepare-list-verify");
    let mut registry = ToolRegistry::default();
    WebwrightToolContributor.contribute(&mut registry).unwrap();
    let ctx = webwright_context(root.clone());
    let prepare = registry.get(WEBWRIGHT_PREPARE_WORKSPACE_TOOL).unwrap();
    let result = prepare
        .execute(
            ctx.clone(),
            call(
                WEBWRIGHT_PREPARE_WORKSPACE_TOOL,
                json!({ "task": "Open fixture page", "taskId": "fixture" }),
            ),
        )
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(root.join(".roder/webwright/fixture/plan.md").exists());

    let list = registry.get(WEBWRIGHT_LIST_ARTIFACTS_TOOL).unwrap();
    let listed = list
        .execute(
            ctx,
            call(
                WEBWRIGHT_LIST_ARTIFACTS_TOOL,
                json!({ "workspace": ".roder/webwright/fixture" }),
            ),
        )
        .await
        .unwrap();
    assert!(!listed.is_error);
    assert_eq!(
        listed.data["webwright"]["workspace"]["latestRun"],
        serde_json::Value::Null
    );
}

#[tokio::test]
async fn helper_tools_reject_workspace_escapes() {
    let root = tempdir("reject-escapes");
    let mut registry = ToolRegistry::default();
    WebwrightToolContributor.contribute(&mut registry).unwrap();
    let ctx = webwright_context(root.clone());
    let prepare = registry.get(WEBWRIGHT_PREPARE_WORKSPACE_TOOL).unwrap();

    let err = prepare
        .execute(
            ctx.clone(),
            call(
                WEBWRIGHT_PREPARE_WORKSPACE_TOOL,
                json!({
                    "task": "Open fixture page",
                    "outputDir": "../outside"
                }),
            ),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("workspace root"), "{err}");

    let list = registry.get(WEBWRIGHT_LIST_ARTIFACTS_TOOL).unwrap();
    let err = list
        .execute(
            ctx,
            call(
                WEBWRIGHT_LIST_ARTIFACTS_TOOL,
                json!({
                    "workspace": root.join("../outside").display().to_string()
                }),
            ),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("workspace root"), "{err}");
}

#[tokio::test]
async fn helper_tools_run_script_and_read_log_tail() {
    let root = tempdir("run-script");
    let mut registry = ToolRegistry::default();
    WebwrightToolContributor.contribute(&mut registry).unwrap();
    let ctx = webwright_context(root.clone());
    registry
        .get(WEBWRIGHT_PREPARE_WORKSPACE_TOOL)
        .unwrap()
        .execute(
            ctx.clone(),
            call(
                WEBWRIGHT_PREPARE_WORKSPACE_TOOL,
                json!({ "task": "Open fixture page", "taskId": "fixture" }),
            ),
        )
        .await
        .unwrap();
    std::fs::write(
        root.join(".roder/webwright/fixture/final_script.py"),
        "mkdir -p screenshots\nprintf 'token=abc123\\n'\nprintf png > screenshots/final_execution_001_ok.png\nprintf 'Authorization: Bearer abc123\\nstep 1 action: ok\\nfinal datum: Fixture Heading\\n' > final_script_log.txt\n",
    )
    .unwrap();
    std::fs::write(
        root.join(".roder/webwright/fixture/plan.md"),
        "# Critical Points\n- [x] CP1: Complete the requested Webwright task: Open fixture page\n",
    )
    .unwrap();

    let lint = registry
        .get(WEBWRIGHT_LINT_SCRIPT_TOOL)
        .unwrap()
        .execute(
            ctx.clone(),
            call(
                WEBWRIGHT_LINT_SCRIPT_TOOL,
                json!({ "workspace": ".roder/webwright/fixture" }),
            ),
        )
        .await
        .unwrap();
    assert!(
        lint.is_error,
        "shell fixture intentionally lacks __main__ guard"
    );

    let run = registry
        .get(WEBWRIGHT_RUN_SCRIPT_TOOL)
        .unwrap()
        .execute(
            ctx.clone(),
            call(
                WEBWRIGHT_RUN_SCRIPT_TOOL,
                json!({
                    "workspace": ".roder/webwright/fixture",
                    "python": "sh",
                    "timeoutSeconds": 5
                }),
            ),
        )
        .await
        .unwrap();
    assert!(!run.is_error, "{}", run.text);
    assert_eq!(run.data["webwright"]["runId"], 1);
    assert_eq!(
        run.data["webwright"]["stdout"],
        "[redacted sensitive Webwright output line]"
    );

    let tail = registry
        .get(WEBWRIGHT_READ_LOG_TAIL_TOOL)
        .unwrap()
        .execute(
            ctx.clone(),
            call(
                WEBWRIGHT_READ_LOG_TAIL_TOOL,
                json!({ "workspace": ".roder/webwright/fixture", "maxLines": 3 }),
            ),
        )
        .await
        .unwrap();
    assert_eq!(
        tail.data["webwright"]["lines"][0],
        "[redacted sensitive Webwright output line]"
    );
    assert_eq!(tail.data["webwright"]["lines"][1], "step 1 action: ok");
    assert_eq!(
        tail.data["webwright"]["lines"][2],
        "final datum: Fixture Heading"
    );

    let verification = registry
        .get(WEBWRIGHT_VERIFY_RUN_TOOL)
        .unwrap()
        .execute(
            ctx.clone(),
            call(
                WEBWRIGHT_VERIFY_RUN_TOOL,
                json!({ "workspace": ".roder/webwright/fixture" }),
            ),
        )
        .await
        .unwrap();
    assert!(!verification.is_error, "{verification:?}");

    let summary = registry
        .get(WEBWRIGHT_SUMMARIZE_VERIFICATION_TOOL)
        .unwrap()
        .execute(
            ctx,
            call(
                WEBWRIGHT_SUMMARIZE_VERIFICATION_TOOL,
                json!({ "workspace": ".roder/webwright/fixture" }),
            ),
        )
        .await
        .unwrap();
    assert!(!summary.is_error);
}

#[cfg(unix)]
#[tokio::test]
async fn run_script_uses_managed_setup_python_when_no_override_is_passed() {
    let root = tempdir("run-script-managed-python");
    let roder_home = root.join("roder-home");
    let managed_python = roder_home.join("python/webwright/venv/bin/python");
    let _env = set_roder_config_dir(&roder_home).await;
    write_managed_python(&managed_python);
    std::fs::write(
        roder_home.join("python/webwright/setup.json"),
        serde_json::json!({
            "version": 1,
            "roderHome": roder_home.display().to_string(),
            "runtimeDir": roder_home.join("python/webwright").display().to_string(),
            "python": managed_python.display().to_string(),
            "browser": "firefox",
            "installedAt": "test"
        })
        .to_string(),
    )
    .unwrap();

    let mut registry = ToolRegistry::default();
    WebwrightToolContributor.contribute(&mut registry).unwrap();
    let ctx = webwright_context(root.clone());
    registry
        .get(WEBWRIGHT_PREPARE_WORKSPACE_TOOL)
        .unwrap()
        .execute(
            ctx.clone(),
            call(
                WEBWRIGHT_PREPARE_WORKSPACE_TOOL,
                json!({ "task": "Open fixture page", "taskId": "fixture" }),
            ),
        )
        .await
        .unwrap();
    std::fs::write(
        root.join(".roder/webwright/fixture/final_script.py"),
        "mkdir -p screenshots\nprintf png > screenshots/final_execution_001_ok.png\nprintf 'step 1 action: ok\\nfinal datum: managed runtime\\n' > final_script_log.txt\n",
    )
    .unwrap();

    let run = registry
        .get(WEBWRIGHT_RUN_SCRIPT_TOOL)
        .unwrap()
        .execute(
            ctx,
            call(
                WEBWRIGHT_RUN_SCRIPT_TOOL,
                json!({
                    "workspace": ".roder/webwright/fixture",
                    "timeoutSeconds": 5
                }),
            ),
        )
        .await
        .unwrap();

    assert!(!run.is_error, "{}", run.text);
    assert_eq!(
        std::fs::read_to_string(
            root.join(".roder/webwright/fixture/final_runs/run_001/final_script_log.txt")
        )
        .unwrap(),
        "step 1 action: ok\nfinal datum: managed runtime\n"
    );
}

#[cfg(unix)]
struct RoderConfigEnvGuard {
    _guard: tokio::sync::MutexGuard<'static, ()>,
    previous_config: Option<std::ffi::OsString>,
    previous_data: Option<std::ffi::OsString>,
}

#[cfg(unix)]
async fn set_roder_config_dir(path: &std::path::Path) -> RoderConfigEnvGuard {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    let guard = LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    let previous_config = std::env::var_os(roder_config::RODER_CONFIG_DIR_ENV);
    let previous_data = std::env::var_os(roder_config::RODER_DATA_DIR_ENV);
    unsafe {
        std::env::set_var(roder_config::RODER_CONFIG_DIR_ENV, path);
        std::env::set_var(roder_config::RODER_DATA_DIR_ENV, path);
    }
    RoderConfigEnvGuard {
        _guard: guard,
        previous_config,
        previous_data,
    }
}

#[cfg(unix)]
impl Drop for RoderConfigEnvGuard {
    fn drop(&mut self) {
        unsafe {
            if let Some(value) = self.previous_config.as_ref() {
                std::env::set_var(roder_config::RODER_CONFIG_DIR_ENV, value);
            } else {
                std::env::remove_var(roder_config::RODER_CONFIG_DIR_ENV);
            }
            if let Some(value) = self.previous_data.as_ref() {
                std::env::set_var(roder_config::RODER_DATA_DIR_ENV, value);
            } else {
                std::env::remove_var(roder_config::RODER_DATA_DIR_ENV);
            }
        }
    }
}

#[tokio::test]
async fn run_script_requires_process_runner_handle() {
    let root = tempdir("missing-process-runner");
    let mut registry = ToolRegistry::default();
    WebwrightToolContributor.contribute(&mut registry).unwrap();
    let ctx = webwright_context_without_process_runner(root);
    let err = registry
        .get(WEBWRIGHT_RUN_SCRIPT_TOOL)
        .unwrap()
        .execute(
            ctx,
            call(
                WEBWRIGHT_RUN_SCRIPT_TOOL,
                json!({ "workspace": ".roder/webwright/fixture" }),
            ),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("process runner"), "{err}");
}

#[cfg(unix)]
fn write_managed_python(path: &std::path::Path) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        path,
        r#"#!/bin/sh
if [ "$1" = "-c" ]; then
  exit 0
fi
exec sh "$@"
"#,
    )
    .unwrap();
    let mut permissions = std::fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).unwrap();
}

#[tokio::test]
async fn helper_tools_register_duplicate_names_as_errors() {
    let mut registry = ToolRegistry::default();
    WebwrightToolContributor.contribute(&mut registry).unwrap();
    let err = WebwrightToolContributor
        .contribute(&mut registry)
        .unwrap_err()
        .to_string();
    assert!(err.contains("already registered"), "{err}");

    let names = [
        WEBWRIGHT_PREPARE_WORKSPACE_TOOL,
        WEBWRIGHT_ALLOCATE_RUN_TOOL,
        WEBWRIGHT_LINT_SCRIPT_TOOL,
        WEBWRIGHT_RUN_SCRIPT_TOOL,
        WEBWRIGHT_LIST_ARTIFACTS_TOOL,
        WEBWRIGHT_READ_LOG_TAIL_TOOL,
        WEBWRIGHT_VERIFY_RUN_TOOL,
        WEBWRIGHT_SUMMARIZE_VERIFICATION_TOOL,
    ];
    for name in names {
        assert!(registry.get(name).is_some(), "missing {name}");
    }
}
