use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use roder_api::dynamic_workflows::{
    WorkflowRunLimits, WorkflowScript, WorkflowScriptSource, WorkflowScriptSourceKind,
};
use roder_commands::{
    CommandDirectory, CommandSource, CommandsRegistry, CommandsRegistryOptions,
    WorkflowCommandDirectory, WorkflowCommandSaveRequest, save_workflow_command,
    workflow_command_arguments,
};
use time::OffsetDateTime;

static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

#[test]
fn workflow_commands_load_from_user_and_workspace_with_workspace_priority() {
    let dir = tempdir("workflow_commands_load_from_user_and_workspace_with_workspace_priority");
    let user = dir.join("user-workflows");
    let workspace = dir.join("workspace/.agents/workflows");
    write(
        &user.join("audit.workflow.js"),
        &workflow_source("audit", "User audit workflow", "target"),
    );
    write(
        &workspace.join("audit.workflow.js"),
        &workflow_source("audit", "Workspace audit workflow", "target"),
    );

    let registry = CommandsRegistry::from_directories_with_workflows(
        std::iter::empty::<CommandDirectory>(),
        [
            WorkflowCommandDirectory {
                root: user,
                source: CommandSource::User,
            },
            WorkflowCommandDirectory {
                root: workspace,
                source: CommandSource::Workspace,
            },
        ],
        CommandsRegistryOptions {
            include_builtins: false,
            allow_builtin_override: false,
        },
    )
    .unwrap();

    let spec = registry.get("audit").unwrap();
    assert_eq!(
        spec.description.as_deref(),
        Some("Workspace audit workflow")
    );
    assert_eq!(spec.source, CommandSource::Workspace);
    assert!(spec.workflow.is_some());
    assert_eq!(
        workflow_command_arguments(spec, "payment service")
            .unwrap()
            .unwrap()["target"],
        "payment service"
    );
}

#[test]
fn workflow_commands_need_explicit_confirmation_to_override_builtin() {
    let dir = tempdir("workflow_commands_need_explicit_confirmation_to_override_builtin");
    let workspace = dir.join(".agents/workflows");
    write(
        &workspace.join("deep-research.workflow.js"),
        &workflow_source("deep-research", "Project research", "question"),
    );

    let err = CommandsRegistry::from_directories_with_workflows(
        std::iter::empty::<CommandDirectory>(),
        [WorkflowCommandDirectory {
            root: workspace.clone(),
            source: CommandSource::Workspace,
        }],
        CommandsRegistryOptions::default(),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("duplicate command `deep-research`"), "{err}");
    assert!(err.contains("built-in command"), "{err}");

    let registry = CommandsRegistry::from_directories_with_workflows(
        std::iter::empty::<CommandDirectory>(),
        [WorkflowCommandDirectory {
            root: workspace,
            source: CommandSource::Workspace,
        }],
        CommandsRegistryOptions {
            include_builtins: true,
            allow_builtin_override: true,
        },
    )
    .unwrap();
    let spec = registry.get("deep-research").unwrap();
    assert_eq!(spec.source, CommandSource::Workspace);
    assert_eq!(registry.override_audits().len(), 1);
    assert_eq!(registry.override_audits()[0].name, "deep-research");
}

#[test]
fn extension_workflow_commands_must_use_extension_namespace() {
    let dir = tempdir("extension_workflow_commands_must_use_extension_namespace");
    let extension = dir.join("extension");
    write(
        &extension.join("review.workflow.js"),
        &workflow_source("review", "Extension review", "target"),
    );

    let err = CommandsRegistry::from_directories_with_workflows(
        std::iter::empty::<CommandDirectory>(),
        [WorkflowCommandDirectory {
            root: extension.clone(),
            source: CommandSource::Extension {
                extension_id: "demo".to_string(),
            },
        }],
        CommandsRegistryOptions {
            include_builtins: false,
            allow_builtin_override: false,
        },
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("must use namespace `ext.demo.`"), "{err}");

    fs::remove_file(extension.join("review.workflow.js")).unwrap();
    write(
        &extension.join("namespaced.workflow.js"),
        &workflow_source("ext.demo.review", "Extension review", "target"),
    );
    let registry = CommandsRegistry::from_directories_with_workflows(
        std::iter::empty::<CommandDirectory>(),
        [WorkflowCommandDirectory {
            root: extension,
            source: CommandSource::Extension {
                extension_id: "demo".to_string(),
            },
        }],
        CommandsRegistryOptions {
            include_builtins: false,
            allow_builtin_override: false,
        },
    )
    .unwrap();
    assert!(registry.get("ext.demo.review").is_some());
}

#[test]
fn save_workflow_command_refuses_overwrite_without_explicit_confirmation() {
    let dir = tempdir("save_workflow_command_refuses_overwrite_without_explicit_confirmation");
    let target = dir.join(".roder/workflows");
    let script = workflow_script("saved-review", "Saved review", "target");

    let path = save_workflow_command(WorkflowCommandSaveRequest {
        target_dir: target.clone(),
        script: script.clone(),
        overwrite: false,
    })
    .unwrap();
    assert!(path.ends_with("saved-review.workflow.js"));

    let err = save_workflow_command(WorkflowCommandSaveRequest {
        target_dir: target.clone(),
        script: script.clone(),
        overwrite: false,
    })
    .unwrap_err()
    .to_string();
    assert!(err.contains("save workflow command"), "{err}");

    save_workflow_command(WorkflowCommandSaveRequest {
        target_dir: target,
        script,
        overwrite: true,
    })
    .unwrap();
}

fn workflow_script(name: &str, description: &str, argument_name: &str) -> WorkflowScript {
    let body = workflow_source(name, description, argument_name);
    WorkflowScript {
        script_id: format!("script-{name}"),
        name: name.to_string(),
        description: Some(description.to_string()),
        source: WorkflowScriptSource {
            kind: WorkflowScriptSourceKind::Generated,
            path: None,
            command_name: Some(name.to_string()),
            extension_id: None,
        },
        hash: format!("hash-{name}"),
        host_api_version: 1,
        arguments_schema: serde_json::json!({
            "type": "object",
            "required": [argument_name],
            "properties": {
                argument_name: { "type": "string" }
            }
        }),
        body: Some(body),
        limits: WorkflowRunLimits::default(),
        created_at: OffsetDateTime::UNIX_EPOCH,
        updated_at: OffsetDateTime::UNIX_EPOCH,
    }
}

fn workflow_source(name: &str, description: &str, argument_name: &str) -> String {
    format!(
        r#"
workflow.define({{
  name: "{name}",
  description: "{description}",
  hostApiVersion: 1,
  argumentsSchema: {{
    type: "object",
    required: ["{argument_name}"],
    properties: {{
      "{argument_name}": {{ type: "string" }}
    }}
  }},
  phases: ["run"],
  limits: {{ maxAgentsPerRun: 2 }}
}}, async (ctx) => {{
  ctx.phase.start("run");
  const value = String(ctx.run.arguments.{argument_name} || "");
  const result = await ctx.agents.run("worker", {{
    prompt: `Handle ${{value}}`,
    output: `done:${{value}}`
  }});
  return ctx.report.markdown(result.output);
}});
"#
    )
}

fn tempdir(name: &str) -> PathBuf {
    let unique = TEMP_COUNTER.fetch_add(1, Ordering::SeqCst);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "roder-commands-{name}-{}-{nanos}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn write(path: &PathBuf, content: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}
