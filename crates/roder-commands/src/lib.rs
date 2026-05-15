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
            "memory",
            "Inspect relevant project and user memory.",
            "Surface relevant memory for the current workspace and task.",
        ),
    ]
    .into_iter()
    .map(|(name, description, body)| CommandSpec {
        name: name.to_string(),
        description: Some(description.to_string()),
        argument_hint: None,
        allowed_tools: Vec::new(),
        model: None,
        agent: None,
        include: CommandInclude::default(),
        body: body.to_string(),
        source: CommandSource::BuiltIn,
        path: None,
    })
    .collect()
}
