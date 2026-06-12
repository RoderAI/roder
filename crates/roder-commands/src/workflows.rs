use std::{
    fs, panic,
    path::{Path, PathBuf},
    sync::OnceLock,
    thread,
};

use anyhow::{Context, Result, bail};
use roder_api::dynamic_workflows::{
    WorkflowScript, WorkflowScriptSource, WorkflowScriptSourceKind,
};
use roder_dynamic_workflows::{
    WorkflowRuntimeOptions, deep_research_workflow_source, parse_workflow_definition,
    workflow_script_hash,
};
use time::OffsetDateTime;

use crate::spec::{
    CommandInclude, CommandSource, CommandSpec, WorkflowCommandSpec, structured_workflow_arguments,
};

const BUILT_IN_WORKFLOW_PARSE_STACK_BYTES: usize = 32 * 1024 * 1024;

static BUILT_IN_WORKFLOW_COMMANDS: OnceLock<Vec<CommandSpec>> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowCommandDirectory {
    pub root: PathBuf,
    pub source: CommandSource,
}

#[derive(Debug, Clone)]
pub struct WorkflowCommandSaveRequest {
    pub target_dir: PathBuf,
    pub script: WorkflowScript,
    pub overwrite: bool,
}

pub fn built_in_workflow_commands() -> Vec<CommandSpec> {
    BUILT_IN_WORKFLOW_COMMANDS
        .get_or_init(build_built_in_workflow_commands)
        .clone()
}

fn build_built_in_workflow_commands() -> Vec<CommandSpec> {
    let handle = thread::Builder::new()
        .name("roder-built-in-workflow-commands".to_string())
        .stack_size(BUILT_IN_WORKFLOW_PARSE_STACK_BYTES)
        .spawn(|| {
            vec![
                workflow_command_from_source(
                    deep_research_workflow_source(),
                    CommandSource::BuiltIn,
                    None,
                )
                .expect("built-in deep research workflow is valid"),
            ]
        })
        .expect("spawn built-in workflow parser thread");
    match handle.join() {
        Ok(commands) => commands,
        Err(payload) => panic::resume_unwind(payload),
    }
}

pub fn load_workflow_command_file(
    path: impl AsRef<Path>,
    source: CommandSource,
) -> Result<CommandSpec> {
    let path = path.as_ref();
    if !is_workflow_script_file(path) {
        bail!(
            "{}: workflow command definitions must use the .workflow.js extension",
            path.display()
        );
    }
    let source_text =
        fs::read_to_string(path).with_context(|| format!("read workflow {}", path.display()))?;
    workflow_command_from_source(&source_text, source, Some(path.to_path_buf()))
}

pub fn scan_workflow_directory(root: &Path, source: CommandSource) -> Result<Vec<CommandSpec>> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    if !root.is_dir() {
        bail!(
            "workflow command source {} is not a directory",
            root.display()
        );
    }

    let mut files = Vec::new();
    collect_workflow_script_files(root, &mut files)?;
    files.sort();

    let mut commands = Vec::with_capacity(files.len());
    for path in files {
        commands.push(load_workflow_command_file(&path, source.clone())?);
    }
    Ok(commands)
}

pub fn save_workflow_command(request: WorkflowCommandSaveRequest) -> Result<PathBuf> {
    let body = request.script.body.as_deref().ok_or_else(|| {
        anyhow::anyhow!(
            "workflow script `{}` cannot be saved because the script body is missing",
            request.script.name
        )
    })?;
    parse_workflow_definition(body, &WorkflowRuntimeOptions::default()).map_err(|err| {
        anyhow::anyhow!(
            "workflow script `{}` failed validation before save: {}",
            request.script.name,
            err
        )
    })?;

    fs::create_dir_all(&request.target_dir).with_context(|| {
        format!(
            "create workflow command dir {}",
            request.target_dir.display()
        )
    })?;
    let path = request.target_dir.join(format!(
        "{}.workflow.js",
        command_file_stem(&request.script.name)?
    ));
    let mut options = fs::OpenOptions::new();
    options.write(true);
    if request.overwrite {
        options.create(true).truncate(true);
    } else {
        options.create_new(true);
    }
    use std::io::Write;
    let mut file = options
        .open(&path)
        .with_context(|| format!("save workflow command {}", path.display()))?;
    file.write_all(body.as_bytes())
        .with_context(|| format!("write workflow command {}", path.display()))?;
    Ok(path)
}

pub fn workflow_command_from_source(
    source_text: &str,
    source: CommandSource,
    path: Option<PathBuf>,
) -> Result<CommandSpec> {
    let definition = parse_workflow_definition(source_text, &WorkflowRuntimeOptions::default())
        .map_err(|err| match &path {
            Some(path) => anyhow::anyhow!("{}: {}", path.display(), err),
            None => anyhow::anyhow!("{err}"),
        })?;
    let hash = workflow_script_hash(source_text);
    let now = OffsetDateTime::now_utc();
    let script = WorkflowScript {
        script_id: format!(
            "workflow-command:{}:{}",
            definition.name,
            hash_prefix(&hash)
        ),
        name: definition.name.clone(),
        description: definition.description.clone(),
        source: workflow_script_source(&source, &definition.name, path.as_ref()),
        hash: hash.clone(),
        host_api_version: definition.host_api_version,
        arguments_schema: definition.arguments_schema.clone(),
        body: Some(source_text.to_string()),
        limits: definition.limits,
        created_at: now,
        updated_at: now,
    };

    Ok(workflow_command_from_script(script, source, path))
}

pub fn workflow_command_from_script(
    script: WorkflowScript,
    source: CommandSource,
    path: Option<PathBuf>,
) -> CommandSpec {
    let argument_hint = argument_hint_from_schema(&script.arguments_schema);
    let description = script.description.clone().or_else(|| {
        Some(format!(
            "Run the {} dynamic workflow.",
            script.name.replace(['-', '_'], " ")
        ))
    });
    let body = format!(
        "Run dynamic workflow command `{}` using its saved workflow script. Parse command arguments with the workflow argument schema and pass the resulting JSON object to the workflow runner.\n\nArguments:\n{{{{arguments}}}}",
        script.name
    );
    CommandSpec {
        name: script.name.clone(),
        description,
        argument_hint,
        allowed_tools: Vec::new(),
        model: None,
        agent: None,
        include: CommandInclude::default(),
        feature_skill_bindings: Vec::new(),
        body,
        workflow: Some(WorkflowCommandSpec {
            script_id: script.script_id,
            script_hash: script.hash,
            host_api_version: script.host_api_version,
            arguments_schema: script.arguments_schema,
            body: script.body,
        }),
        source,
        path,
    }
}

pub fn workflow_command_arguments(
    spec: &CommandSpec,
    arguments: &str,
) -> Result<Option<serde_json::Value>> {
    let Some(workflow) = &spec.workflow else {
        return Ok(None);
    };
    structured_workflow_arguments(&workflow.arguments_schema, arguments).map(Some)
}

fn workflow_script_source(
    source: &CommandSource,
    command_name: &str,
    path: Option<&PathBuf>,
) -> WorkflowScriptSource {
    let (kind, extension_id) = match source {
        CommandSource::BuiltIn => (WorkflowScriptSourceKind::BuiltIn, None),
        CommandSource::User => (WorkflowScriptSourceKind::User, None),
        CommandSource::Workspace => (WorkflowScriptSourceKind::Workspace, None),
        CommandSource::Extension { extension_id } => (
            WorkflowScriptSourceKind::Extension,
            Some(extension_id.clone()),
        ),
        CommandSource::Package { package_id } => (
            WorkflowScriptSourceKind::Extension,
            Some(package_id.clone()),
        ),
    };
    WorkflowScriptSource {
        kind,
        path: path.map(|path| path.display().to_string()),
        command_name: Some(command_name.to_string()),
        extension_id,
    }
}

fn argument_hint_from_schema(schema: &serde_json::Value) -> Option<String> {
    let properties = schema
        .get("properties")
        .and_then(serde_json::Value::as_object)?;
    if let Some(required) = schema.get("required").and_then(serde_json::Value::as_array) {
        let fields = required
            .iter()
            .filter_map(serde_json::Value::as_str)
            .filter(|field| properties.contains_key(*field))
            .collect::<Vec<_>>();
        if fields.len() == 1 {
            return Some(format!("<{}>", fields[0]));
        }
        if !fields.is_empty() {
            return Some(format!(
                "{{{}}}",
                fields
                    .iter()
                    .map(|field| format!("\"{field}\": ..."))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    if properties.len() == 1 {
        return properties.keys().next().map(|field| format!("<{field}>"));
    }
    Some("<json-args>".to_string())
}

fn collect_workflow_script_files(root: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    let mut entries = fs::read_dir(root)
        .with_context(|| format!("read workflow command directory {}", root.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .with_context(|| format!("read workflow command directory {}", root.display()))?;
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let path = entry.path();
        let file_type = entry
            .file_type()
            .with_context(|| format!("read workflow command file type {}", path.display()))?;
        if file_type.is_dir() {
            collect_workflow_script_files(&path, files)?;
        } else if file_type.is_file() && is_workflow_script_file(&path) {
            files.push(path);
        }
    }
    Ok(())
}

fn is_workflow_script_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".workflow.js"))
}

fn command_file_stem(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        bail!("workflow command name must not be empty");
    }
    if name
        .chars()
        .any(|ch| ch == '/' || ch == '\\' || ch.is_control())
    {
        bail!("workflow command name `{name}` cannot be used as a file name");
    }
    Ok(name.to_string())
}

fn hash_prefix(hash: &str) -> &str {
    hash.get(..12).unwrap_or(hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_deep_research_is_a_workflow_command() {
        let command = built_in_workflow_commands()
            .into_iter()
            .find(|command| command.name == roder_dynamic_workflows::DEEP_RESEARCH_COMMAND_NAME)
            .expect("deep research command");

        assert_eq!(command.source, CommandSource::BuiltIn);
        assert_eq!(command.argument_hint.as_deref(), Some("<question>"));
        assert!(command.workflow.is_some());
        assert_eq!(
            workflow_command_arguments(&command, "What changed in agent orchestration?")
                .unwrap()
                .unwrap()["question"],
            "What changed in agent orchestration?"
        );
    }
}
