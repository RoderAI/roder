pub mod expand;
mod frontmatter;
pub mod loader;
pub mod registry;
pub mod spec;
pub mod template;

pub use expand::{
    CommandExpansion, CommandExpansionOptions, CommandExpansionRequest, ShellRunner, UrlFetcher,
    expand_command,
};
pub use loader::load_command_file;
pub use registry::{
    CommandDirectory, CommandOverrideAudit, CommandsRegistry, CommandsRegistryOptions,
    ExtensionCommandDirectory,
};
pub use spec::{CommandInclude, CommandSource, CommandSpec, FileInclude, ShellInclude, UrlInclude};

pub fn built_in_commands() -> Vec<CommandSpec> {
    [
        (
            "init",
            "Create or refresh project instructions for this workspace.",
            "Inspect the workspace and draft concise project instructions.",
        ),
        (
            "clear",
            "Clear the visible conversation state.",
            "Clear the active conversation display.",
        ),
        (
            "compact",
            "Summarize the current thread and continue with a smaller context.",
            "Compact the current thread while preserving the working state.",
        ),
        (
            "help",
            "Show available commands and common workflows.",
            "List available commands and explain the current command surface.",
        ),
        (
            "goal",
            "Create a new active goal from an objective.",
            "Create or inspect the current active goal.",
        ),
        (
            "retry",
            "Resubmit the last user message.",
            "Retry the last user prompt.",
        ),
        (
            "model",
            "Show or change the active model.",
            "Show the active model and available model choices.",
        ),
        (
            "agents",
            "List configured subagents.",
            "List available subagents and their intended uses.",
        ),
        (
            "tasks",
            "Open the background task monitor.",
            "Show background tasks and their recent output.",
        ),
        (
            "memory",
            "Inspect relevant project and user memory.",
            "Surface relevant memory for the current workspace and task.",
        ),
        (
            "commit",
            "Create a scoped git commit.",
            "Create a scoped git commit using the bound commit skill. Inspect the current git state, include only requested changes, and report the commit outcome.",
        ),
        (
            "marketplace",
            "Manage plugin marketplaces.",
            "Use the Roder marketplace app-server methods to list default marketplaces, install one or all defaults, add local marketplaces, refresh catalogs, and search de-duplicated plugin results. Interpret arguments as a marketplace command, for example: list, install-default all, add <id> --kind <kind> --path <path>, refresh <id>, search <query>, or show <marketplace-id> <plugin-id>.",
        ),
        (
            "plugin",
            "Preview, install, list, disable, or uninstall marketplace plugins.",
            "Use the Roder plugin marketplace app-server methods to preview installs, install selected plugin variants, install all de-duplicated variants, list installed variants, disable an installed variant, or uninstall by variant key. Interpret arguments as a plugin command, for example: preview <marketplace-id> <plugin-id>, install <marketplace-id> <plugin-id> [--all-variants], install-all <marketplace-id> <plugin-id>, list, disable <variant-key>, or uninstall <variant-key>.",
        ),
    ]
    .into_iter()
    .map(|(name, description, body)| CommandSpec {
        name: name.to_string(),
        description: Some(description.to_string()),
        argument_hint: match name {
            "marketplace" => Some(
                "list|install-default|add|remove|refresh|search|show [args]".to_string(),
            ),
            "plugin" => {
                Some("preview|install|install-all|list|disable|uninstall [args]".to_string())
            }
            _ => None,
        },
        allowed_tools: Vec::new(),
        model: None,
        agent: None,
        include: CommandInclude::default(),
        feature_skill_bindings: feature_skill_bindings(name),
        body: body.to_string(),
        source: CommandSource::BuiltIn,
        path: None,
    })
    .collect()
}

fn feature_skill_bindings(name: &str) -> Vec<roder_api::skills::FeatureSkillBinding> {
    match name {
        "commit" => vec![roder_api::skills::FeatureSkillBinding {
            feature_id: "command:commit".to_string(),
            skill_selector: roder_api::skills::SkillSelector::Name {
                name: "commit".to_string(),
            },
            required: true,
            activation_reason: roder_api::skills::SkillActivationReason::FeatureBinding,
        }],
        _ => Vec::new(),
    }
}
