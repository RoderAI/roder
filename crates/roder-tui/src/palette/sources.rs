use roder_api::inference::HostedWebSearchMode;
use roder_api::marketplace::MarketplaceDescriptor;
use roder_api::policy_mode::PolicyMode;
use roder_protocol::{
    AgentDescriptor, CommandDescriptor, Item, ProvidersListResult, RunnersListResult,
    SearchIndexSettings, ShellSettings, SpeechProvidersListResult, Thread, WebSearchSettings,
};

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

pub fn workflow_import_source() -> StaticPaletteSource {
    StaticPaletteSource::new(
        "workflow-imports",
        "Workflow Imports",
        vec![
            (
                PaletteItem {
                    id: "workflow-scan".to_string(),
                    title: "Workflow: scan repository".to_string(),
                    subtitle: Some(
                        "Detect AGENTS.md, skills, MCP, hooks, commands, and plugins".to_string(),
                    ),
                    keywords: vec![
                        "workflow".to_string(),
                        "import".to_string(),
                        "agents".to_string(),
                        "mcp".to_string(),
                    ],
                    icon: Some('W'),
                },
                PaletteAction::InsertComposerText(
                    "Scan this repository for workflow imports and show a preview.".to_string(),
                ),
            ),
            (
                PaletteItem {
                    id: "workflow-preview".to_string(),
                    title: "Workflow: import preview".to_string(),
                    subtitle: Some(
                        "Review source attribution, conflicts, and approval-gated side effects"
                            .to_string(),
                    ),
                    keywords: vec![
                        "workflow".to_string(),
                        "preview".to_string(),
                        "hooks".to_string(),
                        "skills".to_string(),
                    ],
                    icon: Some('W'),
                },
                PaletteAction::InsertComposerText(
                    "Show the workflow import preview with conflicts and redacted values."
                        .to_string(),
                ),
            ),
            (
                PaletteItem {
                    id: "workflow-enable".to_string(),
                    title: "Workflow: enable import".to_string(),
                    subtitle: Some(
                        "Enable a selected passive import or approval-gated item by id".to_string(),
                    ),
                    keywords: vec![
                        "workflow".to_string(),
                        "enable".to_string(),
                        "import".to_string(),
                    ],
                    icon: Some('W'),
                },
                PaletteAction::InsertComposerText(
                    "Enable workflow import ITEM_ID after showing any required approvals."
                        .to_string(),
                ),
            ),
            (
                PaletteItem {
                    id: "workflow-refresh-remove".to_string(),
                    title: "Workflow: refresh or remove".to_string(),
                    subtitle: Some(
                        "Refresh stale imports or remove an enabled workflow item".to_string(),
                    ),
                    keywords: vec![
                        "workflow".to_string(),
                        "refresh".to_string(),
                        "remove".to_string(),
                        "ignore".to_string(),
                    ],
                    icon: Some('W'),
                },
                PaletteAction::InsertComposerText(
                    "Refresh workflow imports and offer ignore or remove actions for stale items."
                        .to_string(),
                ),
            ),
        ],
    )
}

pub fn marketplace_source(marketplaces: &[MarketplaceDescriptor]) -> StaticPaletteSource {
    let mut entries = vec![
        (
            PaletteItem {
                id: "marketplaces-install-all-defaults".to_string(),
                title: "Install all default marketplaces".to_string(),
                subtitle: Some("Claude, Cursor, and Codex marketplace metadata".to_string()),
                keywords: vec![
                    "marketplace".to_string(),
                    "plugins".to_string(),
                    "claude".to_string(),
                    "cursor".to_string(),
                    "codex".to_string(),
                ],
                icon: Some('◆'),
            },
            PaletteAction::InsertComposerText("/marketplace install-default all".to_string()),
        ),
        (
            PaletteItem {
                id: "marketplaces-install-claude-default".to_string(),
                title: "Install Claude marketplace".to_string(),
                subtitle: Some("Add Anthropic Claude Plugins Official metadata".to_string()),
                keywords: vec![
                    "marketplace".to_string(),
                    "plugins".to_string(),
                    "claude".to_string(),
                    "anthropic".to_string(),
                ],
                icon: Some('C'),
            },
            PaletteAction::InsertComposerText("/marketplace install-default anthropic".to_string()),
        ),
        (
            PaletteItem {
                id: "marketplaces-install-cursor-default".to_string(),
                title: "Install Cursor marketplace".to_string(),
                subtitle: Some("Add Cursor marketplace metadata".to_string()),
                keywords: vec![
                    "marketplace".to_string(),
                    "plugins".to_string(),
                    "cursor".to_string(),
                ],
                icon: Some('C'),
            },
            PaletteAction::InsertComposerText("/marketplace install-default cursor".to_string()),
        ),
        (
            PaletteItem {
                id: "marketplaces-install-codex-default".to_string(),
                title: "Install Codex marketplace".to_string(),
                subtitle: Some("Add Codex plugin directory metadata".to_string()),
                keywords: vec![
                    "marketplace".to_string(),
                    "plugins".to_string(),
                    "codex".to_string(),
                    "openai".to_string(),
                ],
                icon: Some('C'),
            },
            PaletteAction::InsertComposerText("/marketplace install-default codex".to_string()),
        ),
        (
            PaletteItem {
                id: "marketplaces-add-custom".to_string(),
                title: "Add custom marketplace".to_string(),
                subtitle: Some("Seed marketplace add with id, kind, and source".to_string()),
                keywords: vec![
                    "marketplace".to_string(),
                    "custom".to_string(),
                    "add".to_string(),
                    "github".to_string(),
                    "local".to_string(),
                ],
                icon: Some('+'),
            },
            PaletteAction::InsertComposerText(
                "/marketplace add <id> --kind auto --github owner/repo".to_string(),
            ),
        ),
        (
            PaletteItem {
                id: "marketplaces-browse-plugins".to_string(),
                title: "Browse installable plugins".to_string(),
                subtitle: Some("Open the plugin marketplace browser".to_string()),
                keywords: vec![
                    "marketplace".to_string(),
                    "plugin".to_string(),
                    "install".to_string(),
                    "browse".to_string(),
                ],
                icon: Some('P'),
            },
            PaletteAction::OpenPluginBrowser,
        ),
        (
            PaletteItem {
                id: "marketplaces-search".to_string(),
                title: "Search plugin marketplaces".to_string(),
                subtitle: Some(
                    "List de-duplicated plugin results across installed marketplaces".to_string(),
                ),
                keywords: vec![
                    "marketplace".to_string(),
                    "plugin".to_string(),
                    "dedupe".to_string(),
                    "search".to_string(),
                ],
                icon: Some('⌕'),
            },
            PaletteAction::InsertComposerText("/marketplace search ".to_string()),
        ),
        (
            PaletteItem {
                id: "marketplaces-remove".to_string(),
                title: "Remove marketplace".to_string(),
                subtitle: Some(
                    "Remove a custom marketplace or disable a baked-in default".to_string(),
                ),
                keywords: vec![
                    "marketplace".to_string(),
                    "remove".to_string(),
                    "disable".to_string(),
                    "custom".to_string(),
                ],
                icon: Some('-'),
            },
            PaletteAction::InsertComposerText("/marketplace remove <marketplace-id>".to_string()),
        ),
        (
            PaletteItem {
                id: "plugins-preview-install".to_string(),
                title: "Preview plugin install".to_string(),
                subtitle: Some("Show source, component hints, capabilities, and risk".to_string()),
                keywords: vec![
                    "plugin".to_string(),
                    "preview".to_string(),
                    "install".to_string(),
                    "marketplace".to_string(),
                ],
                icon: Some('P'),
            },
            PaletteAction::InsertComposerText(
                "/plugin preview <marketplace-id> <plugin-id>".to_string(),
            ),
        ),
        (
            PaletteItem {
                id: "plugins-install-selected".to_string(),
                title: "Install selected plugin variant".to_string(),
                subtitle: Some("Install one marketplace/provider variant".to_string()),
                keywords: vec![
                    "plugin".to_string(),
                    "install".to_string(),
                    "variant".to_string(),
                    "marketplace".to_string(),
                ],
                icon: Some('I'),
            },
            PaletteAction::InsertComposerText(
                "/plugin install <marketplace-id> <plugin-id>".to_string(),
            ),
        ),
        (
            PaletteItem {
                id: "plugins-install-all-variants".to_string(),
                title: "Install all plugin variants".to_string(),
                subtitle: Some("Install every provider copy for a de-duped plugin row".to_string()),
                keywords: vec![
                    "plugin".to_string(),
                    "install".to_string(),
                    "all".to_string(),
                    "variants".to_string(),
                ],
                icon: Some('A'),
            },
            PaletteAction::InsertComposerText(
                "/plugin install-all <marketplace-id> <plugin-id>".to_string(),
            ),
        ),
        (
            PaletteItem {
                id: "plugins-list-installed".to_string(),
                title: "List installed plugins".to_string(),
                subtitle: Some("Show installed plugin variants and cache paths".to_string()),
                keywords: vec![
                    "plugin".to_string(),
                    "installed".to_string(),
                    "list".to_string(),
                    "cache".to_string(),
                ],
                icon: Some('L'),
            },
            PaletteAction::InsertComposerText("/plugin list".to_string()),
        ),
        (
            PaletteItem {
                id: "plugins-disable-installed".to_string(),
                title: "Disable installed plugin".to_string(),
                subtitle: Some(
                    "Deactivate an installed variant without deleting cache data".to_string(),
                ),
                keywords: vec![
                    "plugin".to_string(),
                    "disable".to_string(),
                    "installed".to_string(),
                    "variant".to_string(),
                ],
                icon: Some('D'),
            },
            PaletteAction::InsertComposerText("/plugin disable <variant-key>".to_string()),
        ),
        (
            PaletteItem {
                id: "plugins-uninstall-installed".to_string(),
                title: "Uninstall plugin".to_string(),
                subtitle: Some("Remove an installed variant record".to_string()),
                keywords: vec![
                    "plugin".to_string(),
                    "uninstall".to_string(),
                    "remove".to_string(),
                    "installed".to_string(),
                ],
                icon: Some('-'),
            },
            PaletteAction::InsertComposerText("/plugin uninstall <variant-key>".to_string()),
        ),
    ];

    entries.extend(marketplaces.iter().map(|marketplace| {
        (
            PaletteItem {
                id: format!("marketplace-{}", marketplace.id),
                title: format!("Marketplace: {}", marketplace.display_name),
                subtitle: Some(format!(
                    "{} · {:?} · {:?}",
                    marketplace.id, marketplace.kind, marketplace.state
                )),
                keywords: vec![
                    "marketplace".to_string(),
                    "plugins".to_string(),
                    marketplace.id.clone(),
                    marketplace.display_name.clone(),
                    format!("{:?}", marketplace.kind).to_ascii_lowercase(),
                ],
                icon: Some('◇'),
            },
            PaletteAction::InsertComposerText(format!("/marketplace refresh {}", marketplace.id)),
        )
    }));

    StaticPaletteSource::new("marketplaces", "Marketplaces", entries)
}

pub fn media_source() -> StaticPaletteSource {
    StaticPaletteSource::new(
        "media",
        "Media",
        vec![
            (
                PaletteItem {
                    id: "media-image".to_string(),
                    title: "Generate image".to_string(),
                    subtitle: Some(
                        "Use media_generate_image and save a Roder artifact".to_string(),
                    ),
                    keywords: vec![
                        "media".to_string(),
                        "imagegen".to_string(),
                        "artifact".to_string(),
                    ],
                    icon: Some('I'),
                },
                PaletteAction::InsertComposerText("/imagegen ".to_string()),
            ),
            (
                PaletteItem {
                    id: "media-video".to_string(),
                    title: "Generate video".to_string(),
                    subtitle: Some(
                        "Use media_generate_video and save a Roder artifact".to_string(),
                    ),
                    keywords: vec![
                        "media".to_string(),
                        "videogen".to_string(),
                        "artifact".to_string(),
                    ],
                    icon: Some('V'),
                },
                PaletteAction::InsertComposerText("/videogen ".to_string()),
            ),
        ],
    )
}

pub fn memories_source() -> StaticPaletteSource {
    StaticPaletteSource::new(
        "memories",
        "Memories",
        vec![
            (
                PaletteItem {
                    id: "memory-query".to_string(),
                    title: "Memory: query".to_string(),
                    subtitle: Some("Search project and global memories".to_string()),
                    keywords: vec!["memory".to_string(), "query".to_string()],
                    icon: Some('M'),
                },
                PaletteAction::InsertComposerText("/memory query ".to_string()),
            ),
            (
                PaletteItem {
                    id: "memory-save".to_string(),
                    title: "Memory: save".to_string(),
                    subtitle: Some("Save a project or global memory".to_string()),
                    keywords: vec!["memory".to_string(), "save".to_string()],
                    icon: Some('M'),
                },
                PaletteAction::InsertComposerText("/memory save ".to_string()),
            ),
            (
                PaletteItem {
                    id: "memory-providers".to_string(),
                    title: "Memory: providers".to_string(),
                    subtitle: Some("Select embedding provider and model".to_string()),
                    keywords: vec!["memory".to_string(), "embedding".to_string()],
                    icon: Some('M'),
                },
                PaletteAction::InsertComposerText("/memory providers list".to_string()),
            ),
        ],
    )
}

pub fn remote_source() -> StaticPaletteSource {
    StaticPaletteSource::new(
        "remote",
        "Remote",
        vec![(
            PaletteItem {
                id: "remote-control".to_string(),
                title: "Remote control".to_string(),
                subtitle: Some("Open the remote app-server pairing panel".to_string()),
                keywords: vec![
                    "remote".to_string(),
                    "phone".to_string(),
                    "pairing".to_string(),
                    "websocket".to_string(),
                ],
                icon: Some('R'),
            },
            PaletteAction::InsertComposerText("/remote".to_string()),
        )],
    )
}

pub fn roadmap_source() -> StaticPaletteSource {
    StaticPaletteSource::new(
        "roadmaps",
        "Roadmaps",
        vec![
            (
                PaletteItem {
                    id: "roadmap-open".to_string(),
                    title: "Roadmaps: open manager".to_string(),
                    subtitle: Some("Open document-first roadmapping mode".to_string()),
                    keywords: vec![
                        "roadmap".to_string(),
                        "plan".to_string(),
                        "planning".to_string(),
                    ],
                    icon: Some('R'),
                },
                PaletteAction::OpenRoadmapMode,
            ),
            (
                PaletteItem {
                    id: "roadmap-command".to_string(),
                    title: "Roadmaps: open specific plan".to_string(),
                    subtitle: Some("Seed /roadmap so you can add a roadmap/*.md path".to_string()),
                    keywords: vec![
                        "roadmap".to_string(),
                        "plan".to_string(),
                        "file".to_string(),
                    ],
                    icon: Some('R'),
                },
                PaletteAction::InsertComposerText("/roadmap ".to_string()),
            ),
        ],
    )
}

pub fn thread_source(threads: &[Thread]) -> StaticPaletteSource {
    StaticPaletteSource::new(
        "threads",
        "Threads",
        threads
            .iter()
            .map(|thread| {
                let short_id = short_id(&thread.id).to_string();
                let title = thread
                    .name
                    .clone()
                    .filter(|title| !title.trim().is_empty())
                    .or_else(|| (!thread.preview.trim().is_empty()).then(|| thread.preview.clone()))
                    .unwrap_or_else(|| format!("Thread {short_id}"));
                let message_count = thread
                    .turns
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .flat_map(|turn| turn.items.iter())
                    .filter(|item| {
                        matches!(item, Item::UserMessage { .. } | Item::AgentMessage { .. })
                    })
                    .count();
                let subtitle = Some(format!("{} - {} messages", thread.cwd, message_count));
                (
                    PaletteItem {
                        id: thread.id.clone(),
                        title,
                        subtitle,
                        keywords: vec![
                            thread.id.clone(),
                            thread.model_provider.clone(),
                            thread.model.clone(),
                            thread.cwd.clone(),
                        ],
                        icon: Some('#'),
                    },
                    PaletteAction::SwitchThread(thread.id.clone()),
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
        .map(swatch_hint)
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

pub fn settings_source(
    web_search: &WebSearchSettings,
    search_index: &SearchIndexSettings,
    shell: &ShellSettings,
    speech: Option<&SpeechProvidersListResult>,
    active_voice_provider: Option<&str>,
    active_voice_model: Option<&str>,
) -> StaticPaletteSource {
    let search_index_items = [
        (
            true,
            "Instant regex search: Enabled",
            "Use the local fastregex index for grep when it can narrow candidates",
        ),
        (
            false,
            "Instant regex search: Disabled",
            "Always scan files directly for grep",
        ),
    ];
    let web_search_items = [
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
    ];
    let voice_items = speech
        .into_iter()
        .flat_map(|speech| speech.providers.iter())
        .flat_map(|provider| {
            provider.models.iter().map(move |model| {
                let active = active_voice_provider == Some(provider.id.as_str())
                    && active_voice_model == Some(model.id.as_str());
                let active_suffix = if active { " (active)" } else { "" };
                let subtitle = voice_model_subtitle(provider, model);
                (
                    PaletteItem {
                        id: format!("voice:{}:{}", provider.id, model.id),
                        title: format!(
                            "Voice model: {} / {}{active_suffix}",
                            provider.name, model.name
                        ),
                        subtitle,
                        keywords: vec![
                            "voice".to_string(),
                            "speech".to_string(),
                            "dictation".to_string(),
                            "transcribe".to_string(),
                            provider.id.clone(),
                            provider.name.clone(),
                            model.id.clone(),
                            model.name.clone(),
                        ],
                        icon: Some('V'),
                    },
                    PaletteAction::SetVoiceModel {
                        provider: provider.id.clone(),
                        model: model.id.clone(),
                    },
                )
            })
        });

    StaticPaletteSource::new(
        "settings",
        "Settings",
        search_index_items
            .into_iter()
            .map(|(enabled, title, subtitle)| {
                let active_suffix = if enabled == search_index.enabled {
                    " (active)"
                } else {
                    ""
                };
                (
                    PaletteItem {
                        id: format!(
                            "search_index:{}",
                            if enabled { "enabled" } else { "disabled" }
                        ),
                        title: format!("{title}{active_suffix}"),
                        subtitle: Some(subtitle.to_string()),
                        keywords: vec![
                            "instant".to_string(),
                            "regex".to_string(),
                            "search".to_string(),
                            "grep".to_string(),
                            "index".to_string(),
                        ],
                        icon: Some('I'),
                    },
                    PaletteAction::SetSearchIndexEnabled(enabled),
                )
            })
            .chain(web_search_items.into_iter().map(|(mode, title, subtitle)| {
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
            }))
            .chain(shell.options.iter().cloned().map(|choice| {
                let active_suffix = if choice == shell.shell {
                    " (active)"
                } else {
                    ""
                };
                (
                    PaletteItem {
                        id: format!("shell:{choice}"),
                        title: format!("Shell command shell: {choice}{active_suffix}"),
                        subtitle: Some(
                            "Choose the shell used by shell and default exec_command calls"
                                .to_string(),
                        ),
                        keywords: vec![
                            "shell".to_string(),
                            "command".to_string(),
                            "exec".to_string(),
                            "bash".to_string(),
                            "zsh".to_string(),
                            "fish".to_string(),
                        ],
                        icon: Some('$'),
                    },
                    PaletteAction::SetShell(choice),
                )
            }))
            .chain(voice_items)
            .chain(std::iter::once((
                PaletteItem {
                    id: "settings:automations".to_string(),
                    title: "Automations status".to_string(),
                    subtitle: Some(
                        "Read app-server scheduler and run counts without enabling scheduling"
                            .to_string(),
                    ),
                    keywords: vec![
                        "automations".to_string(),
                        "scheduler".to_string(),
                        "runs".to_string(),
                        "status".to_string(),
                    ],
                    icon: Some('A'),
                },
                PaletteAction::ShowAutomationsStatus,
            )))
            .chain(std::iter::once((
                PaletteItem {
                    id: "settings:skills".to_string(),
                    title: "Skills manager".to_string(),
                    subtitle: Some("Manage skill enabled state and exposure".to_string()),
                    keywords: vec![
                        "skills".to_string(),
                        "manager".to_string(),
                        "built-in".to_string(),
                        "exposure".to_string(),
                    ],
                    icon: Some('$'),
                },
                PaletteAction::OpenSkillsManager,
            )))
            .collect(),
    )
}

pub fn runner_source(runners: &RunnersListResult) -> StaticPaletteSource {
    StaticPaletteSource::new(
        "runners",
        "Runners",
        runners
            .providers
            .iter()
            .map(|provider| {
                let active = runners.active.as_ref().is_some_and(|runner| {
                    runner.provider_id == provider.provider_id
                        && runner.destination_id == provider.provider_id
                });
                let suffix = if active { " (active)" } else { "" };
                (
                    PaletteItem {
                        id: provider.provider_id.clone(),
                        title: format!("Runner: {}{suffix}", provider.provider_id),
                        subtitle: Some(runner_capabilities_summary(provider)),
                        keywords: vec!["runner".to_string(), provider.provider_id.clone()],
                        icon: Some('>'),
                    },
                    PaletteAction::SelectRunner {
                        destination_id: provider.provider_id.clone(),
                        provider_id: provider.provider_id.clone(),
                    },
                )
            })
            .collect(),
    )
}

fn runner_capabilities_summary(provider: &roder_protocol::RunnerProviderDescriptor) -> String {
    let capabilities = &provider.capabilities;
    let mut labels = Vec::new();
    if capabilities.command_exec {
        labels.push("commands");
    }
    if capabilities.file_read || capabilities.file_write {
        labels.push("files");
    }
    if capabilities.port_preview {
        labels.push("ports");
    }
    if capabilities.snapshots {
        labels.push("snapshots");
    }
    if labels.is_empty() {
        "No advertised capabilities".to_string()
    } else {
        labels.join(", ")
    }
}

fn voice_model_subtitle(
    provider: &roder_protocol::SpeechProviderDescriptor,
    model: &roder_api::speech::SpeechModelDescriptor,
) -> Option<String> {
    let mut parts = Vec::new();
    if !provider.authenticated {
        parts.push("not authenticated".to_string());
    }
    if let Some(description) = model
        .description
        .as_deref()
        .or(provider.description.as_deref())
        .filter(|description| !description.trim().is_empty())
    {
        parts.push(description.to_string());
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" - "))
    }
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
        HostedWebSearchMode::Cached => "cached",
        HostedWebSearchMode::Live => "live",
    }
}

#[cfg(test)]
mod tests {
    use roder_api::inference::{
        HostedWebSearchMode, InferenceCapabilities, ModelDescriptor, ProviderAuthType,
    };
    use roder_api::marketplace::{
        MarketplaceDescriptor, MarketplaceKind, MarketplaceSource, MarketplaceState,
    };
    use roder_api::remote_runner::RunnerCapabilities;
    use roder_api::speech::{SpeechCapabilities, SpeechModelDescriptor};
    use roder_api::subagents::SubagentPermissionMode;
    use roder_protocol::{
        ProviderDescriptor, RunnerProviderDescriptor, RunnerStatus, SpeechProviderDescriptor,
        SpeechProvidersListResult,
    };

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
    fn settings_source_maps_search_index_and_web_search_modes_to_actions() {
        let source = settings_source(
            &WebSearchSettings {
                mode: HostedWebSearchMode::Cached,
            },
            &SearchIndexSettings { enabled: true },
            &ShellSettings {
                shell: "bash".to_string(),
                options: vec!["zsh".to_string(), "bash".to_string()],
            },
            None,
            None,
            None,
        );
        let entries = source.entries();

        let active_search_index = entries
            .iter()
            .find(|entry| matches!(entry.action, PaletteAction::SetSearchIndexEnabled(true)))
            .expect("enabled search index row");
        assert!(active_search_index.item.title.contains("(active)"));
        assert_eq!(
            entries
                .iter()
                .find(|entry| matches!(entry.action, PaletteAction::SetSearchIndexEnabled(false)))
                .unwrap()
                .action,
            PaletteAction::SetSearchIndexEnabled(false),
        );
        let active_web_search = entries
            .iter()
            .find(|entry| {
                matches!(
                    entry.action,
                    PaletteAction::SetWebSearchMode(HostedWebSearchMode::Cached)
                )
            })
            .expect("cached web search row");
        assert!(active_web_search.item.title.contains("(active)"));
        assert_eq!(
            entries
                .iter()
                .find(|entry| {
                    matches!(
                        entry.action,
                        PaletteAction::SetWebSearchMode(HostedWebSearchMode::Live)
                    )
                })
                .unwrap()
                .action,
            PaletteAction::SetWebSearchMode(HostedWebSearchMode::Live),
        );
        let active_shell = entries
            .iter()
            .find(|entry| entry.action == PaletteAction::SetShell("bash".to_string()))
            .expect("active shell row");
        assert!(active_shell.item.title.contains("(active)"));
        assert_eq!(
            entries.last().unwrap().action,
            PaletteAction::OpenSkillsManager
        );
    }

    #[test]
    fn settings_source_maps_speech_models_to_voice_model_actions() {
        let source = settings_source(
            &WebSearchSettings {
                mode: HostedWebSearchMode::Cached,
            },
            &SearchIndexSettings { enabled: true },
            &ShellSettings {
                shell: "bash".to_string(),
                options: vec!["bash".to_string()],
            },
            Some(&SpeechProvidersListResult {
                providers: vec![SpeechProviderDescriptor {
                    id: "openai-speech".to_string(),
                    name: "OpenAI".to_string(),
                    description: Some("OpenAI speech models".to_string()),
                    auth_type: ProviderAuthType::ApiKey,
                    auth_label: Some("OPENAI_API_KEY".to_string()),
                    authenticated: true,
                    auth_detail: None,
                    recommended: true,
                    sort_order: 0,
                    capabilities: SpeechCapabilities::default(),
                    models: vec![SpeechModelDescriptor {
                        id: "gpt-4o-transcribe".to_string(),
                        name: "GPT-4o Transcribe".to_string(),
                        description: Some("High accuracy transcription".to_string()),
                        capabilities: SpeechCapabilities::default(),
                    }],
                }],
            }),
            Some("openai-speech"),
            Some("gpt-4o-transcribe"),
        );
        let entries = source.entries();
        let voice = entries
            .iter()
            .find(|entry| entry.item.id == "voice:openai-speech:gpt-4o-transcribe")
            .expect("voice model row");

        assert_eq!(
            voice.action,
            PaletteAction::SetVoiceModel {
                provider: "openai-speech".to_string(),
                model: "gpt-4o-transcribe".to_string()
            }
        );
        assert!(voice.item.title.contains("(active)"));
        assert!(voice.item.keywords.contains(&"dictation".to_string()));
    }

    #[test]
    fn settings_source_exposes_read_only_automations_status_action() {
        let source = settings_source(
            &WebSearchSettings {
                mode: HostedWebSearchMode::Cached,
            },
            &SearchIndexSettings { enabled: true },
            &ShellSettings {
                shell: "bash".to_string(),
                options: vec!["zsh".to_string(), "bash".to_string()],
            },
            None,
            None,
            None,
        );
        let entries = source.entries();
        let automations = entries
            .iter()
            .find(|entry| entry.item.id == "settings:automations")
            .expect("automations status palette row");

        assert_eq!(automations.item.title, "Automations status");
        assert_eq!(automations.action, PaletteAction::ShowAutomationsStatus);
        assert!(
            automations
                .item
                .subtitle
                .as_ref()
                .unwrap()
                .contains("without enabling")
        );
    }

    #[test]
    fn remote_source_opens_remote_command() {
        let source = remote_source();
        assert_eq!(source.entries()[0].item.title, "Remote control");
        assert_eq!(
            source.entries()[0].action,
            PaletteAction::InsertComposerText("/remote".to_string())
        );
    }

    #[test]
    fn roadmap_source_opens_mode_and_seeds_specific_plan_command() {
        let source = roadmap_source();
        assert_eq!(source.entries()[0].item.title, "Roadmaps: open manager");
        assert_eq!(source.entries()[0].action, PaletteAction::OpenRoadmapMode);
        assert_eq!(
            source.entries()[1].action,
            PaletteAction::InsertComposerText("/roadmap ".to_string())
        );
    }

    #[test]
    fn marketplace_source_exposes_install_add_search_and_plugin_rows() {
        let source = marketplace_source(&[MarketplaceDescriptor {
            id: "cursor-local".to_string(),
            kind: MarketplaceKind::Cursor,
            display_name: "Cursor Local".to_string(),
            source: MarketplaceSource::LocalPath {
                path: "/tmp/cursor".to_string(),
            },
            homepage: None,
            owner_name: None,
            owner_email: None,
            description: None,
            is_default: false,
            enabled: true,
            state: MarketplaceState::Installed,
            last_refreshed_at: None,
            content_hash: None,
        }]);
        let entries = source.entries();
        let titles = entries
            .iter()
            .map(|entry| entry.item.title.as_str())
            .collect::<Vec<_>>();

        assert!(titles.contains(&"Install Claude marketplace"));
        assert!(titles.contains(&"Add custom marketplace"));
        assert!(titles.contains(&"Browse installable plugins"));
        assert!(titles.contains(&"Remove marketplace"));
        assert!(titles.contains(&"Preview plugin install"));
        assert!(titles.contains(&"Install selected plugin variant"));
        assert!(titles.contains(&"Install all plugin variants"));
        assert!(titles.contains(&"List installed plugins"));
        assert!(titles.contains(&"Disable installed plugin"));
        assert!(titles.contains(&"Uninstall plugin"));
        assert!(titles.iter().any(|title| title.contains("Cursor Local")));
        assert!(entries.iter().any(|entry| entry.action
            == PaletteAction::InsertComposerText(
                "/marketplace add <id> --kind auto --github owner/repo".to_string()
            )));
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

    #[test]
    fn runner_source_maps_providers_to_select_actions() {
        let source = runner_source(&RunnersListResult {
            active: Some(RunnerStatus {
                destination_id: "unix-local".to_string(),
                provider_id: "unix-local".to_string(),
                state: "active".to_string(),
                session_id: Some("runner-1".to_string()),
            }),
            providers: vec![RunnerProviderDescriptor {
                provider_id: "unix-local".to_string(),
                capabilities: RunnerCapabilities {
                    file_read: true,
                    file_write: true,
                    command_exec: true,
                    port_preview: false,
                    snapshots: false,
                    cancellation: true,
                    artifact_export: false,
                    mounts: Default::default(),
                },
            }],
        });

        let entries = source.entries();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].item.title.contains("(active)"));
        assert_eq!(
            entries[0].action,
            PaletteAction::SelectRunner {
                destination_id: "unix-local".to_string(),
                provider_id: "unix-local".to_string()
            }
        );
    }
}
