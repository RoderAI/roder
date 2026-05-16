use roder_api::policy_mode::PolicyMode;
use roder_api::session::SessionMetadata;
use roder_protocol::{AgentDescriptor, CommandDescriptor, ProvidersListResult};

use super::{PaletteAction, PaletteItem, StaticPaletteSource};

pub fn command_source(commands: &[CommandDescriptor]) -> StaticPaletteSource {
    StaticPaletteSource::new(
        "commands",
        "Commands",
        commands
            .iter()
            .map(|command| {
                (
                    PaletteItem {
                        id: command.name.clone(),
                        title: format!("/{}", command.name),
                        subtitle: command.description.clone().or_else(|| {
                            command
                                .argument_hint
                                .as_ref()
                                .map(|hint| format!("Arguments: {hint}"))
                        }),
                        keywords: vec![command.source.clone()]
                            .into_iter()
                            .chain(command.model.clone())
                            .chain(command.agent.clone())
                            .collect(),
                        icon: Some('/'),
                    },
                    PaletteAction::SendCommand(command.name.clone()),
                )
            })
            .collect(),
    )
}

pub fn session_source(sessions: &[SessionMetadata]) -> StaticPaletteSource {
    StaticPaletteSource::new(
        "sessions",
        "Sessions",
        sessions
            .iter()
            .map(|session| {
                let short_id = short_id(&session.thread_id).to_string();
                let title = session
                    .title
                    .clone()
                    .filter(|title| !title.trim().is_empty())
                    .unwrap_or_else(|| format!("Session {short_id}"));
                let subtitle = session
                    .workspace
                    .clone()
                    .or_else(|| session.model.clone())
                    .map(|detail| format!("{detail} - {} messages", session.message_count));
                (
                    PaletteItem {
                        id: session.thread_id.clone(),
                        title,
                        subtitle,
                        keywords: vec![
                            session.thread_id.clone(),
                            session.provider.clone().unwrap_or_default(),
                            session.model.clone().unwrap_or_default(),
                        ],
                        icon: Some('#'),
                    },
                    PaletteAction::SwitchSession(session.thread_id.clone()),
                )
            })
            .collect(),
    )
}

pub fn agent_source(agents: &[AgentDescriptor]) -> StaticPaletteSource {
    StaticPaletteSource::new(
        "agents",
        "Agents",
        agents
            .iter()
            .map(|agent| {
                (
                    PaletteItem {
                        id: agent.agent_type.clone(),
                        title: agent.agent_type.clone(),
                        subtitle: Some(agent.description.clone()),
                        keywords: agent
                            .tools
                            .iter()
                            .cloned()
                            .chain(agent.model.clone())
                            .collect(),
                        icon: Some('@'),
                    },
                    PaletteAction::InsertComposerText(format!(
                        "Use the {} subagent to ",
                        agent.agent_type
                    )),
                )
            })
            .collect(),
    )
}

pub fn model_source(providers: &ProvidersListResult) -> StaticPaletteSource {
    let mut entries = Vec::new();
    for provider in &providers.providers {
        if provider.models.is_empty() {
            let model = providers.active_model.clone();
            entries.push(model_entry(
                &provider.id,
                &model,
                &model,
                provider.description.clone(),
            ));
            continue;
        }
        for model in &provider.models {
            entries.push(model_entry(
                &provider.id,
                &model.id,
                if model.name.is_empty() {
                    &model.id
                } else {
                    &model.name
                },
                provider.description.clone(),
            ));
        }
    }
    StaticPaletteSource::new("models", "Models", entries)
}

pub fn mode_source(active: PolicyMode) -> StaticPaletteSource {
    StaticPaletteSource::new(
        "modes",
        "Modes",
        [
            PolicyMode::Default,
            PolicyMode::AcceptEdits,
            PolicyMode::Plan,
            PolicyMode::Bypass,
        ]
        .into_iter()
        .map(|mode| {
            let label = policy_mode_label(mode);
            let active_suffix = if mode == active { " (active)" } else { "" };
            (
                PaletteItem {
                    id: label.to_string(),
                    title: format!("{label} mode{active_suffix}"),
                    subtitle: Some(policy_mode_description(mode).to_string()),
                    keywords: vec!["policy".to_string(), "mode".to_string()],
                    icon: Some('>'),
                },
                PaletteAction::SetPolicyMode(mode),
            )
        })
        .collect(),
    )
}

fn model_entry(
    provider_id: &str,
    model_id: &str,
    model_label: &str,
    provider_description: Option<String>,
) -> (PaletteItem, PaletteAction) {
    (
        PaletteItem {
            id: format!("{provider_id}:{model_id}"),
            title: format!("{provider_id}/{model_label}"),
            subtitle: provider_description,
            keywords: vec![provider_id.to_string(), model_id.to_string()],
            icon: Some('*'),
        },
        PaletteAction::SwitchModel {
            provider: provider_id.to_string(),
            model: model_id.to_string(),
        },
    )
}

fn short_id(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

fn policy_mode_label(mode: PolicyMode) -> &'static str {
    match mode {
        PolicyMode::Default => "default",
        PolicyMode::AcceptEdits => "accept_edits",
        PolicyMode::Plan => "plan",
        PolicyMode::Bypass => "bypass",
    }
}

fn policy_mode_description(mode: PolicyMode) -> &'static str {
    match mode {
        PolicyMode::Default => "Ask before risky actions",
        PolicyMode::AcceptEdits => "Accept file edits without prompting",
        PolicyMode::Plan => "Plan-only mode",
        PolicyMode::Bypass => "Bypass policy gates",
    }
}

#[cfg(test)]
mod tests {
    use roder_api::inference::{InferenceCapabilities, ModelDescriptor, ProviderAuthType};
    use roder_api::subagents::SubagentPermissionMode;
    use roder_protocol::ProviderDescriptor;

    use super::*;

    #[test]
    fn command_source_runs_command_by_name() {
        let source = command_source(&[CommandDescriptor {
            name: "review".to_string(),
            description: Some("Review code".to_string()),
            argument_hint: None,
            source: "workspace".to_string(),
            model: None,
            agent: None,
            has_shell_includes: false,
            has_url_includes: false,
        }]);

        assert_eq!(source.entries()[0].item.title, "/review");
        assert_eq!(
            source.entries()[0].action,
            PaletteAction::SendCommand("review".to_string())
        );
    }

    #[test]
    fn model_source_maps_provider_models_to_switch_actions() {
        let source = model_source(&ProvidersListResult {
            active_provider: "mock".to_string(),
            active_model: "mock-small".to_string(),
            active_reasoning: "medium".to_string(),
            providers: vec![ProviderDescriptor {
                id: "mock".to_string(),
                name: "Mock".to_string(),
                description: Some("Local".to_string()),
                auth_type: ProviderAuthType::None,
                auth_label: None,
                authenticated: true,
                auth_detail: None,
                recommended: false,
                sort_order: 0,
                capabilities: InferenceCapabilities::text_only(),
                models: vec![ModelDescriptor {
                    id: "mock-small".to_string(),
                    name: "Mock Small".to_string(),
                    context_window: None,
                    default_reasoning: Some("medium".to_string()),
                    supported_reasoning: Vec::new(),
                }],
            }],
        });

        assert_eq!(
            source.entries()[0].action,
            PaletteAction::SwitchModel {
                provider: "mock".to_string(),
                model: "mock-small".to_string()
            }
        );
    }

    #[test]
    fn agent_source_inserts_agent_prompt_seed() {
        let source = agent_source(&[AgentDescriptor {
            agent_type: "explorer".to_string(),
            description: "Read-only explorer".to_string(),
            tools: vec!["read".to_string()],
            model: Some("gpt-test".to_string()),
            permission_mode: SubagentPermissionMode::ReadOnly,
            max_turns: None,
            max_result_chars: None,
        }]);

        assert_eq!(
            source.entries()[0].action,
            PaletteAction::InsertComposerText("Use the explorer subagent to ".to_string())
        );
    }
}
