use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, bail};
use roder_ext_webwright::{
    WEBWRIGHT_TASK_EXECUTOR_ID, WebwrightManifest, WebwrightMode, WebwrightPreparedVisualJudge,
    WebwrightSetupOptions, WebwrightVisualJudgeRecord, WebwrightWorkspace, export_workspace,
    prepare_visual_judge, render_report_text, sanitize_task_id, setup_webwright_runtime,
    store_visual_judge_record, verify_workspace,
};
use roder_protocol::{
    WebwrightArtifactsResult, WebwrightExportParams, WebwrightExportResult,
    WebwrightLatestRunResult, WebwrightPrepareParams, WebwrightPrepareResult,
    WebwrightReportResult, WebwrightRerunParams, WebwrightRerunResult, WebwrightSetupParams,
    WebwrightSetupResult, WebwrightSetupStepResult, WebwrightSubmitParams, WebwrightVerifyResult,
    WebwrightVisualJudgeParams, WebwrightVisualJudgeResult, WebwrightWorkspaceParams,
};
use serde_json::json;

const FINAL_SCRIPT_FILE: &str = "final_script.py";

pub(crate) struct WebwrightRerunPlan {
    pub task_input: serde_json::Value,
    pub run_id: u32,
    pub run_dir: PathBuf,
}

pub(crate) enum WebwrightVisualJudgeStep {
    Ready {
        workspace: WebwrightWorkspace,
        prepared: WebwrightPreparedVisualJudge,
        provider: String,
        model: String,
    },
    Done(WebwrightVisualJudgeResult),
}

pub(crate) fn prepare(
    workspace_root: &Path,
    params: WebwrightPrepareParams,
) -> anyhow::Result<WebwrightPrepareResult> {
    if params.task.trim().is_empty() {
        bail!("webwright task must not be empty");
    }
    let mode = WebwrightMode::parse(params.mode.as_deref().unwrap_or("run"))?;
    let task_id = params
        .task_id
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| sanitize_task_id(&params.task));
    let workspace_path = webwright_workspace_path(
        workspace_root,
        params.output_dir.as_deref(),
        Some(task_id.as_str()),
    )?;
    let manifest = WebwrightManifest::new(
        task_id.clone(),
        params.task,
        mode,
        params.start_url,
        params.browser,
        params.headless.unwrap_or(true),
    );
    let workspace = WebwrightWorkspace::new(&workspace_path);
    workspace.create(&manifest)?;
    workspace.ensure_starter_files(&manifest)?;

    Ok(WebwrightPrepareResult {
        task_id,
        workspace: serde_json::to_value(workspace.summary()?)?,
    })
}

pub(crate) fn setup(params: WebwrightSetupParams) -> anyhow::Result<WebwrightSetupResult> {
    let report = setup_webwright_runtime(WebwrightSetupOptions {
        browser: params.browser,
        python: params.python,
        dry_run: params.dry_run,
    })?;
    Ok(WebwrightSetupResult {
        roder_home: report.roder_home,
        runtime_dir: report.runtime_dir,
        python: report.python,
        browser: report.browser,
        dry_run: report.dry_run,
        installed: report.installed,
        steps: report
            .steps
            .into_iter()
            .map(|step| WebwrightSetupStepResult {
                label: step.label,
                command: step.command,
                status: step.status,
                stdout_tail: step.stdout_tail,
                stderr_tail: step.stderr_tail,
            })
            .collect(),
        message: report.message,
    })
}

pub(crate) fn artifacts(
    workspace_root: &Path,
    params: WebwrightWorkspaceParams,
) -> anyhow::Result<WebwrightArtifactsResult> {
    let workspace = WebwrightWorkspace::new(resolve_webwright_workspace(workspace_root, &params)?);
    Ok(WebwrightArtifactsResult {
        workspace: serde_json::to_value(workspace.summary()?)?,
    })
}

pub(crate) fn latest_run(
    workspace_root: &Path,
    params: WebwrightWorkspaceParams,
) -> anyhow::Result<WebwrightLatestRunResult> {
    let workspace = WebwrightWorkspace::new(resolve_webwright_workspace(workspace_root, &params)?);
    let summary = workspace.summary()?;
    let run = summary
        .latest_run
        .and_then(|latest| summary.runs.iter().find(|run| run.run_id == latest))
        .map(serde_json::to_value)
        .transpose()?;
    Ok(WebwrightLatestRunResult {
        latest_run: summary.latest_run,
        run,
    })
}

pub(crate) fn report(
    workspace_root: &Path,
    params: WebwrightWorkspaceParams,
) -> anyhow::Result<WebwrightReportResult> {
    let workspace = WebwrightWorkspace::new(resolve_webwright_workspace(workspace_root, &params)?);
    let summary = workspace.summary()?;
    let rendered_text = summary.report.as_ref().map(render_report_text);
    Ok(WebwrightReportResult {
        task_definition: summary
            .task_definition
            .map(serde_json::to_value)
            .transpose()?,
        report: summary.report.map(serde_json::to_value).transpose()?,
        rendered_text,
    })
}

pub(crate) fn verify(
    workspace_root: &Path,
    params: WebwrightWorkspaceParams,
) -> anyhow::Result<WebwrightVerifyResult> {
    let workspace_path = resolve_webwright_workspace(workspace_root, &params)?;
    Ok(WebwrightVerifyResult {
        verification: serde_json::to_value(verify_workspace(workspace_path))?,
    })
}

pub(crate) fn export(
    workspace_root: &Path,
    params: WebwrightExportParams,
) -> anyhow::Result<WebwrightExportResult> {
    let workspace_path = resolve_webwright_workspace(
        workspace_root,
        &WebwrightWorkspaceParams {
            workspace: params.workspace,
            workspace_root: params.workspace_root,
        },
    )?;
    let export_dir = scoped_path(
        workspace_root,
        Path::new(&params.output_dir),
        "Webwright exportDir",
    )?;
    let exported = export_workspace(&WebwrightWorkspace::new(workspace_path), export_dir)?;
    Ok(WebwrightExportResult {
        export_dir: exported.export_dir,
        files: exported.files,
        excluded: exported.excluded,
    })
}

pub(crate) fn visual_judge_step(
    workspace_root: &Path,
    params: WebwrightVisualJudgeParams,
    provider: String,
    model: String,
    provider_image_input: bool,
) -> anyhow::Result<WebwrightVisualJudgeStep> {
    let enabled = params.enabled.unwrap_or_else(visual_judge_enabled_from_env);
    let workspace_path = resolve_webwright_workspace(
        workspace_root,
        &WebwrightWorkspaceParams {
            workspace: params.workspace,
            workspace_root: params.workspace_root,
        },
    )?;
    let workspace = WebwrightWorkspace::new(workspace_path);
    let latest_run = workspace.latest_run_id().ok().flatten();
    if !enabled {
        return Ok(WebwrightVisualJudgeStep::Done(visual_judge_result(
            WebwrightVisualJudgeRecord::skipped(
                &workspace,
                params.run_id.or(latest_run),
                provider,
                model,
                "visual judge disabled; pass enabled=true or set RODER_WEBWRIGHT_VISUAL_JUDGE=1",
            ),
        )?));
    }
    if !provider_image_input {
        return Ok(WebwrightVisualJudgeStep::Done(visual_judge_result(
            WebwrightVisualJudgeRecord::skipped(
                &workspace,
                params.run_id.or(latest_run),
                provider,
                model,
                "active provider does not support image input",
            ),
        )?));
    }
    let prepared = match prepare_visual_judge(&workspace, params.run_id) {
        Ok(prepared) => prepared,
        Err(err) => {
            return Ok(WebwrightVisualJudgeStep::Done(visual_judge_result(
                WebwrightVisualJudgeRecord::failed(
                    None,
                    &workspace,
                    params.run_id.or(latest_run),
                    provider,
                    model,
                    err.to_string(),
                ),
            )?));
        }
    };
    Ok(WebwrightVisualJudgeStep::Ready {
        workspace,
        prepared,
        provider,
        model,
    })
}

pub(crate) fn complete_visual_judge(
    prepared: &WebwrightPreparedVisualJudge,
    provider: String,
    model: String,
    response: String,
) -> anyhow::Result<WebwrightVisualJudgeResult> {
    visual_judge_result(WebwrightVisualJudgeRecord::completed(
        prepared, provider, model, response,
    ))
}

pub(crate) fn fail_visual_judge(
    workspace: &WebwrightWorkspace,
    prepared: Option<&WebwrightPreparedVisualJudge>,
    provider: String,
    model: String,
    reason: String,
) -> anyhow::Result<WebwrightVisualJudgeResult> {
    visual_judge_result(WebwrightVisualJudgeRecord::failed(
        prepared, workspace, None, provider, model, reason,
    ))
}

pub(crate) fn submit_input(params: WebwrightSubmitParams) -> anyhow::Result<serde_json::Value> {
    if params.task.trim().is_empty() {
        bail!("webwright task must not be empty");
    }
    Ok(json!({
        "task": params.task,
        "mode": params.mode,
        "startUrl": params.start_url,
        "taskId": params.task_id,
        "browser": params.browser,
        "headless": params.headless,
        "outputDir": params.output_dir,
        "timeoutSeconds": params.timeout_seconds,
    }))
}

pub(crate) fn prepare_rerun(
    workspace_root: &Path,
    params: WebwrightRerunParams,
) -> anyhow::Result<WebwrightRerunPlan> {
    let workspace_path = resolve_webwright_workspace(
        workspace_root,
        &WebwrightWorkspaceParams {
            workspace: params.workspace,
            workspace_root: params.workspace_root,
        },
    )?;
    let workspace = WebwrightWorkspace::new(&workspace_path);
    let final_script = workspace_path.join(FINAL_SCRIPT_FILE);
    if !final_script.is_file() {
        bail!(
            "missing Webwright final_script.py: {}",
            final_script.display()
        );
    }

    let run_id = workspace.next_run_id()?;
    let run_dir = workspace.run_dir(run_id);
    fs::create_dir_all(run_dir.join("screenshots"))
        .with_context(|| format!("create {}", run_dir.display()))?;
    fs::copy(&final_script, run_dir.join(FINAL_SCRIPT_FILE)).with_context(|| {
        format!(
            "copy {} to {}",
            final_script.display(),
            run_dir.join(FINAL_SCRIPT_FILE).display()
        )
    })?;

    if let Some(mut manifest) = workspace.read_manifest()? {
        manifest.latest_run = Some(run_id);
        manifest.verification_state = "pending".to_string();
        workspace.write_manifest(&manifest)?;
    }

    let interpreter = match params.python {
        Some(python) if !python.trim().is_empty() => python,
        _ => {
            let browser = workspace
                .read_manifest()?
                .map(|manifest| manifest.browser)
                .unwrap_or_else(|| "firefox".to_string());
            roder_ext_webwright::preflight_local_dependencies(
                roder_ext_webwright::DependencyCheckMode::Required,
                Some(&browser),
            )?
            .python_command
        }
    };
    Ok(WebwrightRerunPlan {
        task_input: json!({
            "command": interpreter,
            "args": [FINAL_SCRIPT_FILE],
            "cwd": run_dir.display().to_string(),
        }),
        run_id,
        run_dir,
    })
}

pub(crate) fn task_executor_id() -> String {
    WEBWRIGHT_TASK_EXECUTOR_ID.to_string()
}

pub(crate) fn process_executor_id() -> String {
    "process".to_string()
}

pub(crate) fn rerun_result(
    task: roder_api::tasks::TaskHandle,
    plan: WebwrightRerunPlan,
) -> WebwrightRerunResult {
    WebwrightRerunResult {
        task,
        run_id: plan.run_id,
        run_dir: plan.run_dir.display().to_string(),
    }
}

pub(crate) fn workspace_root(
    runtime_workspace: Option<String>,
    requested: Option<String>,
) -> anyhow::Result<PathBuf> {
    let root = requested
        .or(runtime_workspace)
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .context("could not resolve workspace root")?;
    if root
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        bail!("workspace root must not contain parent components");
    }
    Ok(root)
}

fn resolve_webwright_workspace(
    default_root: &Path,
    params: &WebwrightWorkspaceParams,
) -> anyhow::Result<PathBuf> {
    let root = workspace_root(None, params.workspace_root.clone())?;
    let scope = if params.workspace_root.is_some() {
        root.as_path()
    } else {
        default_root
    };
    scoped_path(scope, Path::new(&params.workspace), "Webwright workspace")
}

fn webwright_workspace_path(
    workspace_root: &Path,
    output_dir: Option<&str>,
    task_id: Option<&str>,
) -> anyhow::Result<PathBuf> {
    let selected = output_dir
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(".roder")
                .join("webwright")
                .join(task_id.unwrap_or("webwright-task"))
        });
    scoped_path(workspace_root, &selected, "Webwright outputDir")
}

fn scoped_path(root: &Path, value: &Path, label: &str) -> anyhow::Result<PathBuf> {
    if value
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        bail!("{label} must stay inside the workspace root");
    }
    let resolved = if value.is_absolute() {
        value.to_path_buf()
    } else {
        root.join(value)
    };
    if !resolved.starts_with(root) {
        bail!("{label} must stay inside the workspace root");
    }
    Ok(resolved)
}

fn visual_judge_result(
    record: WebwrightVisualJudgeRecord,
) -> anyhow::Result<WebwrightVisualJudgeResult> {
    Ok(WebwrightVisualJudgeResult {
        visual_judge: serde_json::to_value(store_visual_judge_record(record)?)?,
    })
}

fn visual_judge_enabled_from_env() -> bool {
    std::env::var("RODER_WEBWRIGHT_VISUAL_JUDGE")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "on"))
        .unwrap_or(false)
}
