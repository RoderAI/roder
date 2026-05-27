use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use roder_api::processes::{
    ProcessDescriptor, ProcessOrigin, ProcessState, ProcessStopper, command_summary,
};
use roder_api::remote_runner::{RunnerCommandRequest, RunnerSessionState};
use roder_api::tasks::{
    TaskExecutionContext, TaskExecutionResult, TaskExecutor, TaskOutputStream, TaskSpec,
};
use serde::Deserialize;
use serde_json::json;

use crate::playwright::{DependencyCheckMode, preflight_local_dependencies};
use crate::workspace::{
    WebwrightManifest, WebwrightMode, WebwrightWorkspace, sanitize_task_id, scoped_path,
};

pub const WEBWRIGHT_TASK_EXECUTOR_ID: &str = "webwright.browser_task";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebwrightTaskInput {
    pub task: String,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub start_url: Option<String>,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub browser: Option<String>,
    #[serde(default)]
    pub headless: Option<bool>,
    #[serde(default)]
    pub output_dir: Option<String>,
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct WebwrightTaskExecutor {
    dependency_check: DependencyCheckMode,
}

impl Default for WebwrightTaskExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl WebwrightTaskExecutor {
    pub fn new() -> Self {
        Self {
            dependency_check: DependencyCheckMode::Required,
        }
    }

    pub fn without_dependency_check() -> Self {
        Self {
            dependency_check: DependencyCheckMode::Skipped,
        }
    }
}

#[async_trait::async_trait]
impl TaskExecutor for WebwrightTaskExecutor {
    fn id(&self) -> String {
        WEBWRIGHT_TASK_EXECUTOR_ID.to_string()
    }

    fn spec(&self) -> TaskSpec {
        TaskSpec {
            kind: WEBWRIGHT_TASK_EXECUTOR_ID.to_string(),
            description: "Prepare and run a Webwright browser automation workspace.".to_string(),
            input_schema: json!({
                "type": "object",
                "required": ["task"],
                "properties": {
                    "task": { "type": "string" },
                    "mode": { "type": "string", "enum": ["run", "craft"] },
                    "startUrl": { "type": "string" },
                    "taskId": { "type": "string" },
                    "browser": { "type": "string" },
                    "headless": { "type": "boolean" },
                    "outputDir": { "type": "string" },
                    "timeoutSeconds": { "type": "integer", "minimum": 1 }
                },
                "additionalProperties": false
            }),
            default_timeout_seconds: Some(900),
            metadata: json!({ "category": "browser", "artifactContract": "webwright" }),
        }
    }

    async fn execute(
        &self,
        ctx: TaskExecutionContext,
        input: serde_json::Value,
    ) -> anyhow::Result<TaskExecutionResult> {
        let input: WebwrightTaskInput =
            serde_json::from_value(input).context("deserialize Webwright task input")?;
        if input.task.trim().is_empty() {
            bail!("webwright task must not be empty");
        }
        let mode = WebwrightMode::parse(input.mode.as_deref().unwrap_or("run"))?;
        let task_id = input
            .task_id
            .clone()
            .filter(|id| !id.trim().is_empty())
            .unwrap_or_else(|| sanitize_task_id(&input.task));
        let workspace_root = workspace_root(&ctx, input.output_dir.as_deref(), &task_id)?;
        let report = preflight_local_dependencies(self.dependency_check, input.browser.as_deref())?;
        ctx.output
            .write(
                TaskOutputStream::Log,
                format!("webwright dependencies: {}", report.message),
            )
            .await?;

        let manifest = WebwrightManifest::new(
            task_id.clone(),
            input.task.clone(),
            mode,
            input.start_url.clone(),
            input.browser.clone(),
            input.headless.unwrap_or(true),
        );
        let workspace = WebwrightWorkspace::new(&workspace_root);
        workspace.create(&manifest)?;
        workspace.ensure_starter_files(&manifest)?;

        if let Some(session) = ctx.runner_session.clone() {
            let state = session.state();
            ctx.output
                .write(
                    TaskOutputStream::Log,
                    format!(
                        "webwright remote runner session available: {}/{}",
                        state.destination_id, state.session_id
                    ),
                )
                .await?;
            let result = session
                .run_command(RunnerCommandRequest {
                    command_id: format!("webwright-preflight-{}", ctx.task_id),
                    program: "true".to_string(),
                    args: Vec::new(),
                    cwd: Some(workspace_root.clone()),
                    env: Vec::new(),
                })
                .await
                .context("run Webwright remote preflight")?;
            register_prepared_process(
                &ctx,
                &workspace_root,
                &task_id,
                PreparedProcessRunner::Remote {
                    state,
                    exit_code: result.exit_code,
                },
            )
            .await?;
        } else {
            register_prepared_process(
                &ctx,
                &workspace_root,
                &task_id,
                PreparedProcessRunner::Local,
            )
            .await?;
        }

        let summary = workspace.summary()?;
        ctx.output
            .write(
                TaskOutputStream::Log,
                format!(
                    "webwright workspace prepared at {}",
                    workspace_root.display()
                ),
            )
            .await?;
        Ok(TaskExecutionResult::success(json!({
            "webwright": {
                "taskId": task_id,
                "mode": mode.as_str(),
                "workspace": summary,
                "dependencyReport": report,
                "readyForModelIteration": true
            }
        })))
    }
}

fn workspace_root(
    ctx: &TaskExecutionContext,
    output_dir: Option<&str>,
    task_id: &str,
) -> anyhow::Result<PathBuf> {
    let root = runtime_workspace_root(ctx)?;
    let selected = output_dir
        .filter(|dir| !dir.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".roder").join("webwright").join(task_id));
    scoped_path(&root, selected, "Webwright outputDir")
}

fn runtime_workspace_root(ctx: &TaskExecutionContext) -> anyhow::Result<PathBuf> {
    Ok(ctx
        .workspace_root
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir()?))
}

async fn register_prepared_process(
    ctx: &TaskExecutionContext,
    workspace_root: &Path,
    task_id: &str,
    runner: PreparedProcessRunner,
) -> anyhow::Result<()> {
    let Some(registry) = ctx.process_registry.as_ref() else {
        return Ok(());
    };
    let command = vec![
        "webwright".to_string(),
        "prepare".to_string(),
        task_id.to_string(),
    ];
    registry
        .register_process(
            ProcessDescriptor {
                process_id: format!("webwright-prepare-{}", ctx.task_id),
                origin: ProcessOrigin::BackgroundTask,
                state: ProcessState::Exited {
                    exit_code: runner.exit_code(),
                },
                command: command.clone(),
                command_summary: command_summary(&command),
                cwd: Some(workspace_root.display().to_string()),
                pid: None,
                task_id: Some(ctx.task_id.clone()),
                thread_id: ctx.thread_id.clone(),
                turn_id: ctx.turn_id.clone(),
                runner_destination_id: runner.destination_id(),
                runner_session_id: runner.session_id(),
                stoppable: false,
                started_at: time::OffsetDateTime::now_utc(),
                updated_at: time::OffsetDateTime::now_utc(),
                stdout_tail: None,
                stderr_tail: None,
            },
            None::<std::sync::Arc<dyn ProcessStopper>>,
        )
        .await
        .map(|_| ())
}

enum PreparedProcessRunner {
    Local,
    Remote {
        state: RunnerSessionState,
        exit_code: Option<i32>,
    },
}

impl PreparedProcessRunner {
    fn exit_code(&self) -> Option<i32> {
        match self {
            Self::Local => Some(0),
            Self::Remote { exit_code, .. } => *exit_code,
        }
    }

    fn destination_id(&self) -> Option<String> {
        match self {
            Self::Local => None,
            Self::Remote { state, .. } => Some(state.destination_id.clone()),
        }
    }

    fn session_id(&self) -> Option<String> {
        match self {
            Self::Local => None,
            Self::Remote { state, .. } => Some(state.session_id.clone()),
        }
    }
}

#[cfg(test)]
mod tests;
