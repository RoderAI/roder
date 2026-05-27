use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use roder_api::tools::{ToolCall, ToolExecutionContext, ToolResult, ToolSpec};
use serde::Serialize;
use serde_json::json;
use tokio::process::Command;

use crate::playwright::{DependencyCheckMode, preflight_local_dependencies};
use crate::redaction::{redact_sensitive_line, redact_sensitive_text};
use crate::workspace::{
    FINAL_LOG_FILE, FINAL_SCRIPT_FILE, WebwrightRunSummary, WebwrightWorkspace, scoped_path,
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ScriptLintResult {
    pub(super) passed: bool,
    pub(super) checks: Vec<ScriptLintCheck>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ScriptLintCheck {
    pub(super) id: String,
    pub(super) passed: bool,
    pub(super) message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ScriptRunResult {
    pub(super) run_id: u32,
    pub(super) run_dir: String,
    pub(super) exit_code: Option<i32>,
    pub(super) stdout: String,
    pub(super) stderr: String,
    pub(super) timed_out: bool,
    pub(super) workspace: serde_json::Value,
}

pub(super) fn allocate_next_run(
    workspace: &WebwrightWorkspace,
) -> anyhow::Result<WebwrightRunSummary> {
    let script = workspace.root().join(FINAL_SCRIPT_FILE);
    if !script.is_file() {
        anyhow::bail!("missing Webwright final_script.py: {}", script.display());
    }
    let run_id = workspace.next_run_id()?;
    let run_dir = workspace.run_dir(run_id);
    fs::create_dir_all(run_dir.join("screenshots"))?;
    fs::copy(&script, run_dir.join(FINAL_SCRIPT_FILE))?;
    if let Some(mut manifest) = workspace.read_manifest()? {
        manifest.latest_run = Some(run_id);
        manifest.verification_state = "pending".to_string();
        workspace.write_manifest(&manifest)?;
    }
    workspace
        .summary()?
        .runs
        .into_iter()
        .find(|run| run.run_id == run_id)
        .ok_or_else(|| anyhow::anyhow!("allocated run_{run_id:03} was not summarized"))
}

pub(super) fn lint_script_text(text: &str) -> ScriptLintResult {
    let checks = vec![
        ScriptLintCheck {
            id: "no_full_page_screenshots".to_string(),
            passed: !(text.contains("full_page=True")
                || text.contains("full_page = True")
                || text.contains("fullPage: true")),
            message: "script must not request full-page screenshots".to_string(),
        },
        ScriptLintCheck {
            id: "main_guard".to_string(),
            passed: text.contains("__main__"),
            message: "script should use an import-safe __main__ guard".to_string(),
        },
    ];
    ScriptLintResult {
        passed: checks.iter().all(|check| check.passed),
        checks,
    }
}

pub(super) async fn run_final_script(
    run: &WebwrightRunSummary,
    interpreter: &str,
    timeout_seconds: u64,
) -> anyhow::Result<ScriptRunResult> {
    let run_dir = PathBuf::from(&run.run_dir);
    let run_future = Command::new(interpreter)
        .arg(FINAL_SCRIPT_FILE)
        .current_dir(&run_dir)
        .output();
    let output = match tokio::time::timeout(Duration::from_secs(timeout_seconds), run_future).await
    {
        Ok(output) => output?,
        Err(_) => {
            return Ok(ScriptRunResult {
                run_id: run.run_id,
                run_dir: run.run_dir.clone(),
                exit_code: None,
                stdout: String::new(),
                stderr: format!("timed out after {timeout_seconds}s"),
                timed_out: true,
                workspace: serde_json::Value::Null,
            });
        }
    };
    let stdout = redact_sensitive_text(&String::from_utf8_lossy(&output.stdout));
    let stderr = redact_sensitive_text(&String::from_utf8_lossy(&output.stderr));
    let log_path = run_dir.join(FINAL_LOG_FILE);
    if !log_path.exists() {
        let mut log = String::new();
        if !stdout.is_empty() {
            log.push_str(&stdout);
        }
        if !stderr.is_empty() {
            log.push_str(&stderr);
        }
        fs::write(&log_path, log)?;
    }
    let workspace_root = run_dir
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| anyhow::anyhow!("invalid Webwright run directory {}", run.run_dir))?;
    Ok(ScriptRunResult {
        run_id: run.run_id,
        run_dir: run.run_dir.clone(),
        exit_code: output.status.code(),
        stdout,
        stderr,
        timed_out: false,
        workspace: serde_json::to_value(WebwrightWorkspace::new(workspace_root).summary()?)?,
    })
}

pub(super) fn default_python_for_workspace(
    _ctx: &ToolExecutionContext,
    workspace: &WebwrightWorkspace,
) -> anyhow::Result<String> {
    let browser = workspace
        .read_manifest()?
        .map(|manifest| manifest.browser)
        .unwrap_or_else(|| "firefox".to_string());
    Ok(preflight_local_dependencies(DependencyCheckMode::Required, Some(&browser))?.python_command)
}

pub(super) fn read_tail_lines(path: &Path, max_lines: usize) -> anyhow::Result<Vec<String>> {
    let text = fs::read_to_string(path)?;
    let mut lines = text.lines().map(redact_sensitive_line).collect::<Vec<_>>();
    if lines.len() > max_lines {
        lines = lines.split_off(lines.len() - max_lines);
    }
    Ok(lines)
}

pub(super) fn workspace_tool_spec(name: &str, description: &str) -> ToolSpec {
    ToolSpec {
        name: name.to_string(),
        description: description.to_string(),
        parameters: json!({
            "type": "object",
            "required": ["workspace"],
            "properties": {
                "workspace": { "type": "string" }
            },
            "additionalProperties": false
        }),
    }
}

pub(super) fn workspace_path(
    ctx: &ToolExecutionContext,
    output_dir: Option<&str>,
    task_id: &str,
) -> anyhow::Result<PathBuf> {
    let workspace = ctx.require_workspace()?.workspace_root().ok_or_else(|| {
        anyhow::anyhow!("workspace root is not available for Webwright tool execution")
    })?;
    let relative = output_dir
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".roder").join("webwright").join(task_id));
    scoped_path(&workspace, relative, "Webwright outputDir")
}

pub(super) fn resolve_workspace_arg(
    ctx: &ToolExecutionContext,
    value: &str,
) -> anyhow::Result<PathBuf> {
    let workspace = ctx.require_workspace()?.workspace_root().ok_or_else(|| {
        anyhow::anyhow!("workspace root is not available for Webwright tool execution")
    })?;
    scoped_path(&workspace, PathBuf::from(value), "Webwright workspace path")
}

pub(super) fn error_result(call: ToolCall, message: impl Into<String>) -> ToolResult {
    let message = message.into();
    ToolResult {
        id: call.id,
        name: call.name,
        text: message.clone(),
        data: json!({ "error": { "kind": "webwright_validation", "message": message } }),
        is_error: true,
    }
}
