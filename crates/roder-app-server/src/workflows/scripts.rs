use std::path::{Path, PathBuf};

use roder_api::dynamic_workflows::{WorkflowScript, WorkflowScriptSourceKind};
use roder_commands::{
    CommandSource, CommandSpec, built_in_workflow_commands, workflows::scan_workflow_directory,
};
use roder_core::Runtime;
use roder_protocol::{
    WorkflowsScriptsDeleteParams, WorkflowsScriptsDeleteResult, WorkflowsScriptsListParams,
    WorkflowsScriptsListResult, WorkflowsScriptsReadParams, WorkflowsScriptsReadResult,
};

use super::support::script_from_source;

pub(super) fn list_scripts(
    runtime: &Runtime,
    params: WorkflowsScriptsListParams,
) -> anyhow::Result<WorkflowsScriptsListResult> {
    let mut scripts = Vec::new();
    if params.include_builtin {
        for command in built_in_workflow_commands() {
            if let Some(script) = script_from_command(command)? {
                scripts.push(script);
            }
        }
    }
    let config = roder_config::load_config()?;
    let workspace = params
        .workspace
        .map(PathBuf::from)
        .unwrap_or_else(|| runtime.workspace());
    let workflow_dirs = roder_config::dynamic_workflows::resolve_workflow_directories(
        config.dynamic_workflows.as_ref(),
        Some(&workspace),
    );
    scripts.extend(scan_root(
        &workflow_dirs.workspace,
        CommandSource::Workspace,
    )?);
    if params.include_user {
        scripts.extend(scan_root(&workflow_dirs.user, CommandSource::User)?);
    }
    scripts.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then(left.source.path.cmp(&right.source.path))
    });
    Ok(WorkflowsScriptsListResult { scripts })
}

pub(super) fn read_script(
    runtime: &Runtime,
    params: WorkflowsScriptsReadParams,
) -> anyhow::Result<WorkflowsScriptsReadResult> {
    let mut script = find_script(
        runtime,
        params.script_id.as_deref(),
        params.name.as_deref(),
        params.source,
    )?;
    if !params.include_body {
        script.body = None;
    }
    Ok(WorkflowsScriptsReadResult { script })
}

pub(super) fn delete_script(
    runtime: &Runtime,
    params: WorkflowsScriptsDeleteParams,
) -> anyhow::Result<WorkflowsScriptsDeleteResult> {
    let script = find_script(runtime, Some(&params.script_id), None, None)?;
    let Some(path) = script.source.path else {
        anyhow::bail!("workflow script {:?} is not file-backed", params.script_id);
    };
    if params.delete_file {
        std::fs::remove_file(&path)?;
    }
    Ok(WorkflowsScriptsDeleteResult {
        script_id: params.script_id,
        deleted: params.delete_file,
    })
}

fn find_script(
    runtime: &Runtime,
    script_id: Option<&str>,
    name: Option<&str>,
    source: Option<WorkflowScriptSourceKind>,
) -> anyhow::Result<WorkflowScript> {
    let candidates = list_scripts(
        runtime,
        WorkflowsScriptsListParams {
            workspace: None,
            include_user: true,
            include_builtin: true,
        },
    )?
    .scripts;
    candidates
        .into_iter()
        .find(|script| {
            script_id.is_none_or(|id| script.script_id == id)
                && name.is_none_or(|name| script.name == name)
                && source
                    .as_ref()
                    .is_none_or(|source| script.source.kind == *source)
        })
        .ok_or_else(|| anyhow::anyhow!("unknown workflow script"))
}

fn scan_root(root: &Path, source: CommandSource) -> anyhow::Result<Vec<WorkflowScript>> {
    scan_workflow_directory(root, source)?
        .into_iter()
        .map(script_from_command)
        .collect::<anyhow::Result<Vec<_>>>()
        .map(|scripts| scripts.into_iter().flatten().collect())
}

fn script_from_command(command: CommandSpec) -> anyhow::Result<Option<WorkflowScript>> {
    let Some(workflow) = command.workflow else {
        return Ok(None);
    };
    let Some(body) = workflow.body else {
        return Ok(None);
    };
    let kind = match command.source {
        CommandSource::BuiltIn => WorkflowScriptSourceKind::BuiltIn,
        CommandSource::User => WorkflowScriptSourceKind::User,
        CommandSource::Workspace => WorkflowScriptSourceKind::Workspace,
        CommandSource::Extension { .. } => WorkflowScriptSourceKind::Extension,
        CommandSource::Package { .. } => WorkflowScriptSourceKind::Extension,
    };
    script_from_source(
        &body,
        kind,
        command.path.map(|path| path.display().to_string()),
        Some(command.name),
    )
    .map(Some)
}
