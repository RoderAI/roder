use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use roder_api::{context::ContextBlockKind, policy_mode::PolicyMode, skills::SkillExposure};
use roder_commands::{
    CommandExpansionOptions, CommandExpansionRequest, CommandInclude, CommandSource, CommandSpec,
    FileInclude, ShellInclude, ShellRunner, UrlFetcher, UrlInclude, built_in_commands,
    expand_command,
};
use roder_skills::{SkillConfigRule, SkillRegistry, SkillRegistryOptions};

#[test]
fn expand_substitutes_arguments_and_default_values() {
    let dir = tempdir("expand_substitutes_arguments_and_default_values");
    let spec = command("review", r#"Review {{arguments|default("correctness")}}"#);

    let without_args = expand(&spec, "", &dir, CommandExpansionOptions::default()).unwrap();
    assert_eq!(without_args.message, "Review correctness");

    let with_args = expand(&spec, "api", &dir, CommandExpansionOptions::default()).unwrap();
    assert_eq!(with_args.message, "Review api");
}

#[test]
fn expand_resolves_file_include_as_context_reference_and_block() {
    let dir = tempdir("expand_resolves_file_include_as_context_reference_and_block");
    fs::write(dir.join("CLAUDE.md"), "project guide").unwrap();
    let mut spec = command("review", "Guide:\n{{include.files.CLAUDE_md}}");
    spec.include.files.push(FileInclude {
        id: None,
        path: "CLAUDE.md".to_string(),
        optional: false,
    });

    let result = expand(&spec, "", &dir, CommandExpansionOptions::default()).unwrap();
    assert_eq!(
        result.message,
        "Guide:\n[context:command.review.files.CLAUDE_md]"
    );
    assert_eq!(result.context_blocks.len(), 1);
    assert_eq!(
        result.context_blocks[0].kind,
        ContextBlockKind::RetrievedDocument
    );
    assert_eq!(result.context_blocks[0].text, "project guide");
    assert_eq!(
        result.context_blocks[0].metadata["include_kind"].as_str(),
        Some("file")
    );
}

#[test]
fn expand_rejects_missing_template_include() {
    let dir = tempdir("expand_rejects_missing_template_include");
    let spec = command("review", "{{include.files.missing}}");

    let err = expand(&spec, "", &dir, CommandExpansionOptions::default())
        .unwrap_err()
        .to_string();
    assert!(err.contains("unknown include template key"), "{err}");
}

#[test]
fn expand_truncates_oversize_file_include() {
    let dir = tempdir("expand_truncates_oversize_file_include");
    fs::write(dir.join("big.txt"), "abcdef").unwrap();
    let mut spec = command("review", "{{include.files.big_txt}}");
    spec.include.files.push(FileInclude {
        id: None,
        path: "big.txt".to_string(),
        optional: false,
    });
    let options = CommandExpansionOptions {
        max_include_bytes: 3,
        ..CommandExpansionOptions::default()
    };

    let result = expand(&spec, "", &dir, options).unwrap();
    assert_eq!(result.message, "[context:command.review.files.big_txt]");
    assert_eq!(result.context_blocks[0].text, "abc");
    assert_eq!(
        result.context_blocks[0].metadata["truncated"].as_bool(),
        Some(true)
    );
}

#[test]
fn expand_blocks_shell_includes_by_default_and_in_plan_mode() {
    let dir = tempdir("expand_blocks_shell_includes_by_default_and_in_plan_mode");
    let mut spec = command("review", "{{include.shell.diff}}");
    spec.include.shell.push(ShellInclude {
        id: Some("diff".to_string()),
        command: "git diff".to_string(),
        timeout_seconds: None,
    });

    let err = expand(&spec, "", &dir, CommandExpansionOptions::default())
        .unwrap_err()
        .to_string();
    assert!(err.contains("shell includes are disabled"), "{err}");

    let options = CommandExpansionOptions {
        allow_shell_includes: true,
        policy_mode: PolicyMode::Plan,
        ..CommandExpansionOptions::default()
    };
    let err = expand_command(CommandExpansionRequest {
        spec: &spec,
        arguments: "",
        workspace_root: &dir,
        options,
        shell_runner: Some(&FakeShell),
        url_fetcher: None,
        skill_registry: None,
    })
    .unwrap_err()
    .to_string();
    assert!(err.contains("blocked by active policy mode"), "{err}");
}

#[test]
fn expand_runs_shell_include_when_enabled() {
    let dir = tempdir("expand_runs_shell_include_when_enabled");
    let mut spec = command("review", "{{include.shell.diff}}");
    spec.include.shell.push(ShellInclude {
        id: Some("diff".to_string()),
        command: "git diff".to_string(),
        timeout_seconds: Some(7),
    });
    let options = CommandExpansionOptions {
        allow_shell_includes: true,
        ..CommandExpansionOptions::default()
    };

    let result = expand_command(CommandExpansionRequest {
        spec: &spec,
        arguments: "",
        workspace_root: &dir,
        options,
        shell_runner: Some(&FakeShell),
        url_fetcher: None,
        skill_registry: None,
    })
    .unwrap();
    assert_eq!(result.message, "[context:command.review.shell.diff]");
    assert_eq!(result.context_blocks[0].text, "shell:git diff:7");
    assert_eq!(result.context_blocks[0].kind, ContextBlockKind::Environment);
}

#[test]
fn expand_blocks_url_includes_by_default_and_by_host_allowlist() {
    let dir = tempdir("expand_blocks_url_includes_by_default_and_by_host_allowlist");
    let mut spec = command("review", "{{include.urls.docs}}");
    spec.include.urls.push(UrlInclude {
        id: Some("docs".to_string()),
        url: "https://docs.example/path".to_string(),
        optional: false,
    });

    let err = expand(&spec, "", &dir, CommandExpansionOptions::default())
        .unwrap_err()
        .to_string();
    assert!(err.contains("URL includes are disabled"), "{err}");

    let options = CommandExpansionOptions {
        allow_url_includes: true,
        allowed_url_hosts: vec!["other.example".to_string()],
        ..CommandExpansionOptions::default()
    };
    let err = expand_command(CommandExpansionRequest {
        spec: &spec,
        arguments: "",
        workspace_root: &dir,
        options,
        shell_runner: None,
        url_fetcher: Some(&FakeUrl),
        skill_registry: None,
    })
    .unwrap_err()
    .to_string();
    assert!(err.contains("not in the allowlist"), "{err}");
}

#[test]
fn expand_fetches_url_include_when_enabled() {
    let dir = tempdir("expand_fetches_url_include_when_enabled");
    let mut spec = command("review", "{{include.urls.docs}}");
    spec.include.urls.push(UrlInclude {
        id: Some("docs".to_string()),
        url: "https://docs.example/path".to_string(),
        optional: false,
    });
    let options = CommandExpansionOptions {
        allow_url_includes: true,
        allowed_url_hosts: vec!["docs.example".to_string()],
        ..CommandExpansionOptions::default()
    };

    let result = expand_command(CommandExpansionRequest {
        spec: &spec,
        arguments: "",
        workspace_root: &dir,
        options,
        shell_runner: None,
        url_fetcher: Some(&FakeUrl),
        skill_registry: None,
    })
    .unwrap();
    assert_eq!(result.message, "[context:command.review.urls.docs]");
    assert_eq!(
        result.context_blocks[0].text,
        "url:https://docs.example/path:5:65536"
    );
    assert_eq!(
        result.context_blocks[0].metadata["include_kind"].as_str(),
        Some("url")
    );
}

#[test]
fn builtin_snapshot_expansion_includes_direct_only_vcs_snapshot_skill() {
    let dir = tempdir("builtin_snapshot_expansion_includes_direct_only_vcs_snapshot_skill");
    let spec = built_in_commands()
        .into_iter()
        .find(|spec| spec.name == "snapshot")
        .expect("snapshot command");
    let registry = SkillRegistry::load(SkillRegistryOptions::new(&dir));

    let result = expand_command(CommandExpansionRequest {
        spec: &spec,
        arguments: "src/lib.rs",
        workspace_root: &dir,
        options: CommandExpansionOptions::default(),
        shell_runner: None,
        url_fetcher: None,
        skill_registry: Some(&registry),
    })
    .unwrap();

    assert_eq!(result.command_name, "snapshot");
    assert!(result.message.contains("bound VCS snapshot skill"));
    assert!(result.context_blocks.iter().any(|block| {
        block.text.starts_with("<skill name=\"vcs-snapshot\"") && block.text.contains("VCS status")
    }));
}

#[test]
fn required_builtin_snapshot_skill_refuses_when_disabled() {
    let dir = tempdir("required_builtin_snapshot_skill_refuses_when_disabled");
    let spec = built_in_commands()
        .into_iter()
        .find(|spec| spec.name == "snapshot")
        .expect("snapshot command");
    let registry = SkillRegistry::load(SkillRegistryOptions {
        workspace: dir.clone(),
        include_builtins: true,
        roots: Vec::new(),
        workflow_imports: Vec::new(),
        config_rules: vec![SkillConfigRule {
            name: Some("vcs-snapshot".to_string()),
            path: None,
            enabled: Some(false),
            exposure: Some(SkillExposure::DirectOnly),
        }],
    });

    let err = expand_command(CommandExpansionRequest {
        spec: &spec,
        arguments: "",
        workspace_root: &dir,
        options: CommandExpansionOptions::default(),
        shell_runner: None,
        url_fetcher: None,
        skill_registry: Some(&registry),
    })
    .unwrap_err()
    .to_string();

    assert!(err.contains("required skill vcs-snapshot"), "{err}");
    assert!(err.contains("disabled"), "{err}");
}

#[test]
fn builtin_webwright_run_expansion_includes_direct_only_skill_and_task_text() {
    let dir = tempdir("builtin_webwright_run_expansion_includes_direct_only_skill_and_task_text");
    let spec = built_in_commands()
        .into_iter()
        .find(|spec| spec.name == "webwright:run")
        .expect("webwright run command");
    let registry = SkillRegistry::load(SkillRegistryOptions::new(&dir));

    let result = expand_command(CommandExpansionRequest {
        spec: &spec,
        arguments: "Find the cheapest fixture item --start-url http://127.0.0.1:9 --task-id fixture-task --output-dir .roder/webwright/fixture-task $(not-a-shell)",
        workspace_root: &dir,
        options: CommandExpansionOptions::default(),
        shell_runner: None,
        url_fetcher: None,
        skill_registry: Some(&registry),
    })
    .unwrap();

    assert_eq!(result.command_name, "webwright:run");
    assert!(result.message.contains("Mode: run"));
    assert!(result.message.contains("startUrl"));
    assert!(result.message.contains("taskId"));
    assert!(result.message.contains("outputDir"));
    assert!(result
        .message
        .contains("Find the cheapest fixture item --start-url http://127.0.0.1:9 --task-id fixture-task --output-dir .roder/webwright/fixture-task $(not-a-shell)"));
    assert!(result.context_blocks.iter().any(|block| {
        block.text.starts_with("<skill name=\"webwright\"")
            && block.text.contains("webwright.prepare_workspace")
    }));
}

#[test]
fn required_builtin_webwright_skill_refuses_when_disabled() {
    let dir = tempdir("required_builtin_webwright_skill_refuses_when_disabled");
    let spec = built_in_commands()
        .into_iter()
        .find(|spec| spec.name == "webwright:craft")
        .expect("webwright craft command");
    let registry = SkillRegistry::load(SkillRegistryOptions {
        workspace: dir.clone(),
        include_builtins: true,
        roots: Vec::new(),
        workflow_imports: Vec::new(),
        config_rules: vec![SkillConfigRule {
            name: Some("webwright".to_string()),
            path: None,
            enabled: Some(false),
            exposure: Some(SkillExposure::DirectOnly),
        }],
    });

    let err = expand_command(CommandExpansionRequest {
        spec: &spec,
        arguments: "",
        workspace_root: &dir,
        options: CommandExpansionOptions::default(),
        shell_runner: None,
        url_fetcher: None,
        skill_registry: Some(&registry),
    })
    .unwrap_err()
    .to_string();

    assert!(err.contains("required skill webwright"), "{err}");
    assert!(err.contains("disabled"), "{err}");
}

fn expand(
    spec: &CommandSpec,
    arguments: &str,
    workspace_root: &PathBuf,
    options: CommandExpansionOptions,
) -> Result<roder_commands::CommandExpansion> {
    expand_command(CommandExpansionRequest {
        spec,
        arguments,
        workspace_root,
        options,
        shell_runner: None,
        url_fetcher: None,
        skill_registry: None,
    })
}

fn command(name: &str, body: &str) -> CommandSpec {
    CommandSpec {
        name: name.to_string(),
        description: None,
        argument_hint: None,
        allowed_tools: Vec::new(),
        model: None,
        agent: None,
        include: CommandInclude::default(),
        feature_skill_bindings: Vec::new(),
        body: body.to_string(),
        workflow: None,
        source: CommandSource::Workspace,
        path: None,
    }
}

struct FakeShell;

impl ShellRunner for FakeShell {
    fn run_shell(&self, command: &str, timeout_seconds: u64) -> Result<String> {
        Ok(format!("shell:{command}:{timeout_seconds}"))
    }
}

struct FakeUrl;

impl UrlFetcher for FakeUrl {
    fn fetch_url(&self, url: &str, timeout_seconds: u64, max_bytes: usize) -> Result<String> {
        Ok(format!("url:{url}:{timeout_seconds}:{max_bytes}"))
    }
}

static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

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
