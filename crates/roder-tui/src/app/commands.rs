use roder_protocol::CommandDescriptor;

pub(super) fn built_in_command_catalog() -> Vec<CommandDescriptor> {
    [
        (
            "init",
            "Create or refresh project instructions for this workspace.",
        ),
        ("clear", "Clear the visible conversation state."),
        (
            "compact",
            "Summarize the current thread and continue with a smaller context.",
        ),
        ("help", "Show available commands and common workflows."),
        ("model", "Show or change the active model."),
        ("agents", "List configured subagents."),
        ("memory", "Inspect relevant project and user memory."),
    ]
    .into_iter()
    .map(|(name, description)| CommandDescriptor {
        name: name.to_string(),
        description: Some(description.to_string()),
        argument_hint: None,
        source: "built-in".to_string(),
        model: None,
        agent: None,
        has_shell_includes: false,
        has_url_includes: false,
    })
    .collect()
}

pub(super) fn slash_query(input: &str) -> Option<&str> {
    let trimmed = input.trim_start();
    if !trimmed.starts_with('/') || trimmed.starts_with("//") {
        return None;
    }
    let query = trimmed.strip_prefix('/')?;
    if query.contains('\n') {
        return None;
    }
    Some(query)
}

pub(super) fn matching_commands<'a>(
    commands: &'a [CommandDescriptor],
    input: &str,
) -> Vec<&'a CommandDescriptor> {
    let Some(query) = slash_query(input) else {
        return Vec::new();
    };
    let token = query.split_whitespace().next().unwrap_or_default();
    commands
        .iter()
        .filter(|command| command.name.starts_with(token))
        .collect()
}

pub(super) fn should_show_menu(input: &str) -> bool {
    let Some(query) = slash_query(input) else {
        return false;
    };
    !query.contains(char::is_whitespace)
}

pub(super) fn command_invocation(
    input: &str,
    commands: &[CommandDescriptor],
) -> Option<(String, String)> {
    let query = slash_query(input)?;
    let mut parts = query.splitn(2, char::is_whitespace);
    let name = parts.next()?.trim();
    if name.is_empty() || !commands.iter().any(|command| command.name == name) {
        return None;
    }
    Some((
        name.to_string(),
        parts.next().unwrap_or_default().trim().to_string(),
    ))
}

pub(super) fn accepted_completion(
    input: &str,
    commands: &[CommandDescriptor],
    selected: usize,
) -> Option<String> {
    let matches = matching_commands(commands, input);
    let command = matches.get(selected.min(matches.len().saturating_sub(1)))?;
    Some(format!("/{} ", command.name))
}

pub(super) fn command_warning(command: &CommandDescriptor) -> Option<String> {
    if command.agent.is_some() {
        Some("uses subagent".to_string())
    } else if command.model.is_some() {
        Some("changes model".to_string())
    } else if command.has_shell_includes {
        Some("shell gated".to_string())
    } else if command.has_url_includes {
        Some("url gated".to_string())
    } else if command.source.starts_with("extension:") {
        Some(format!("extension command from {}", command.source))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_commands_by_prefix() {
        let commands = sample_commands();
        let matches = matching_commands(&commands, "/re");
        assert_eq!(
            matches
                .iter()
                .map(|command| command.name.as_str())
                .collect::<Vec<_>>(),
            ["review", "refactor"]
        );
    }

    #[test]
    fn tab_completion_inserts_command_name() {
        let commands = sample_commands();
        assert_eq!(
            accepted_completion("/he", &commands, 0).as_deref(),
            Some("/help ")
        );
    }

    #[test]
    fn built_in_catalog_contains_expected_slash_commands() {
        let commands = built_in_command_catalog();
        let names = commands
            .iter()
            .map(|command| command.name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            [
                "init", "clear", "compact", "help", "model", "agents", "memory"
            ]
        );
    }

    #[test]
    fn menu_only_shows_while_typing_command_name() {
        assert!(should_show_menu("/"));
        assert!(should_show_menu("/he"));
        assert!(should_show_menu(" /he"));
        assert!(!should_show_menu("/help "));
        assert!(!should_show_menu("//not-command"));
    }

    #[test]
    fn invocation_ignores_unknown_slash_and_paths() {
        let commands = sample_commands();
        assert_eq!(command_invocation("/missing arg", &commands), None);
        assert_eq!(command_invocation("/Users/pz/file", &commands), None);
        assert_eq!(
            command_invocation("/review api", &commands),
            Some(("review".to_string(), "api".to_string()))
        );
    }

    #[test]
    fn extension_commands_show_warning() {
        let warning = command_warning(&CommandDescriptor {
            name: "ext.lint.review".to_string(),
            description: None,
            argument_hint: None,
            source: "extension:lint".to_string(),
            model: None,
            agent: None,
            has_shell_includes: false,
            has_url_includes: false,
        })
        .unwrap();
        assert!(warning.contains("extension:lint"));
    }

    #[test]
    fn gated_commands_show_warning() {
        let warning = command_warning(&CommandDescriptor {
            name: "review".to_string(),
            description: None,
            argument_hint: None,
            source: "workspace".to_string(),
            model: None,
            agent: None,
            has_shell_includes: true,
            has_url_includes: false,
        })
        .unwrap();
        assert_eq!(warning, "shell gated");
    }

    fn sample_commands() -> Vec<CommandDescriptor> {
        vec![
            CommandDescriptor {
                name: "review".to_string(),
                description: Some("Review code".to_string()),
                argument_hint: Some("[area]".to_string()),
                source: "workspace".to_string(),
                model: None,
                agent: None,
                has_shell_includes: false,
                has_url_includes: false,
            },
            CommandDescriptor {
                name: "refactor".to_string(),
                description: None,
                argument_hint: None,
                source: "user".to_string(),
                model: None,
                agent: None,
                has_shell_includes: false,
                has_url_includes: false,
            },
            CommandDescriptor {
                name: "help".to_string(),
                description: None,
                argument_hint: None,
                source: "built-in".to_string(),
                model: None,
                agent: None,
                has_shell_includes: false,
                has_url_includes: false,
            },
        ]
    }
}
