use roder_api::inference::HostedWebSearchMode;
use roder_api::policy_mode::PolicyMode;
use roder_api::session::SessionMetadata;
use roder_protocol::{AgentDescriptor, CommandDescriptor, ProvidersListResult, WebSearchSettings};

use super::{PaletteAction, PaletteItem, StaticPaletteSource};
use crate::theme::{ThemeEntry, ThemeOverrides};

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
            PolicyMode::AcceptAll,
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

/// Build a palette source that lists every discovered theme. Selecting an
/// entry emits a [`PaletteAction::SetTheme`] which the app handles by reloading
/// the stylesheet live and persisting the choice to `~/.roder/state.toml`.
///
/// `active` is the id of the currently-applied theme (used to label the
/// active row) and is matched case-sensitively against `ThemeEntry::id`.
pub fn theme_source(entries: &[ThemeEntry], active: Option<&str>) -> StaticPaletteSource {
    StaticPaletteSource::new(
        "themes",
        "Themes",
        entries
            .iter()
            .map(|entry| {
                let is_active = active.is_some_and(|a| a == entry.id);
                let active_suffix = if is_active { " (active)" } else { "" };
                let swatch = theme_preview_swatch(&entry.path);
                let subtitle = match swatch {
                    Some(s) => Some(format!("{s}  {}", entry.path.display())),
                    None => Some(entry.path.display().to_string()),
                };
                (
                    PaletteItem {
                        id: entry.id.clone(),
                        title: format!("Theme: {}{active_suffix}", entry.display_name),
                        subtitle,
                        keywords: vec![
                            "theme".to_string(),
                            "css".to_string(),
                            "color".to_string(),
                            entry.id.clone(),
                        ],
                        icon: Some('~'),
                    },
                    PaletteAction::SetTheme(entry.id.clone()),
                )
            })
            .collect(),
    )
}

/// Best-effort 3-cell preview built from a theme's `:root` accent/error/border
/// (or background) variables. Returns a small string of fullblock chars whose
/// hex hints are interpolated into the row subtitle. We deliberately keep
/// this cheap — a parse error returns `None` and the row renders without it.
fn theme_preview_swatch(path: &std::path::Path) -> Option<String> {
    let css = std::fs::read_to_string(path).ok()?;
    let overrides = ThemeOverrides::from_css(&css).ok()?;
    let pick = |name: &str| overrides.color(name);
    let primary = pick("accent")
        .or_else(|| pick("text"))
        .or_else(|| pick("border"));
    let secondary = pick("error").or_else(|| pick("mode-bypass"));
    let tertiary = pick("diff-added").or_else(|| pick("tool"));
    let chunks: Vec<String> = [primary, secondary, tertiary]
        .into_iter()
        .flatten()
        .map(|c| swatch_hint(c))
        .collect();
    if chunks.is_empty() {
        None
    } else {
        Some(format!("[{}]", chunks.join(" ")))
    }
}

fn swatch_hint(color: ratatui::style::Color) -> String {
    use ratatui::style::Color::*;
    match color {
        Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Indexed(n) => format!("ansi{n}"),
        Reset => "reset".to_string(),
        other => format!("{other:?}").to_lowercase(),
    }
}

pub fn settings_source(web_search: &WebSearchSettings) -> StaticPaletteSource {
    StaticPaletteSource::new(
        "settings",
        "Settings",
        [
            (
                HostedWebSearchMode::Cached,
                "Web search provider: Codex cached",
                "Use Codex/OpenAI hosted web search over cached content",
            ),
            (
                HostedWebSearchMode::Live,
                "Web search provider: Codex live",
                "Use Codex/OpenAI hosted web search with live internet access",
            ),
            (
                HostedWebSearchMode::Disabled,
                "Web search provider: Disabled",
                "Do not send the hosted web_search tool to the provider",
            ),
        ]
        .into_iter()
        .map(|(mode, title, subtitle)| {
            let active_suffix = if mode == web_search.mode {
                " (active)"
            } else {
                ""
            };
            (
                PaletteItem {
                    id: format!("web_search:{}", web_search_mode_id(mode)),
                    title: format!("{title}{active_suffix}"),
                    subtitle: Some(subtitle.to_string()),
                    keywords: vec![
                        "web".to_string(),
                        "search".to_string(),
                        "provider".to_string(),
                        web_search_mode_id(mode).to_string(),
                    ],
                    icon: Some('~'),
                },
                PaletteAction::SetWebSearchMode(mode),
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
        PolicyMode::AcceptAll => "accept_all",
        PolicyMode::Plan => "plan",
        PolicyMode::Bypass => "bypass",
    }
}

fn policy_mode_description(mode: PolicyMode) -> &'static str {
    match mode {
        PolicyMode::Default => "Ask before risky actions",
        PolicyMode::AcceptAll => "Accept all tool approvals without prompting",
        PolicyMode::Plan => "Plan-only mode",
        PolicyMode::Bypass => "Bypass policy gates",
    }
}

fn web_search_mode_id(mode: HostedWebSearchMode) -> &'static str {
    match mode {
        HostedWebSearchMode::Disabled => "disabled",
        HostedWebSearchMode::Cached => "codex",
        HostedWebSearchMode::Live => "live",
    }
}

#[cfg(test)]
mod tests {
    use roder_api::inference::{
        HostedWebSearchMode, InferenceCapabilities, ModelDescriptor, ProviderAuthType,
    };
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
    fn settings_source_maps_web_search_modes_to_actions() {
        let source = settings_source(&WebSearchSettings {
            mode: HostedWebSearchMode::Cached,
        });
        let entries = source.entries();

        assert!(entries[0].item.title.contains("(active)"));
        assert_eq!(
            entries[1].action,
            PaletteAction::SetWebSearchMode(HostedWebSearchMode::Live)
        );
    }

    #[test]
    fn theme_source_emits_set_theme_action_per_entry() {
        use std::path::PathBuf;
        let entries = vec![
            ThemeEntry {
                id: "default".to_string(),
                display_name: "default".to_string(),
                path: PathBuf::from("themes/default.css"),
            },
            ThemeEntry {
                id: "midnight".to_string(),
                display_name: "midnight".to_string(),
                path: PathBuf::from("themes/midnight.css"),
            },
        ];
        let source = theme_source(&entries, Some("default"));
        let rows = source.entries();
        assert_eq!(rows.len(), 2);
        assert!(rows[0].item.title.contains("(active)"));
        assert_eq!(
            rows[1].action,
            PaletteAction::SetTheme("midnight".to_string())
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
