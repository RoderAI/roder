pub mod index;
pub mod render;
pub mod skills;
pub mod sources;

use roder_api::events::ThreadId;
use roder_api::inference::HostedWebSearchMode;
use roder_api::policy_mode::PolicyMode;
use roder_api::skills::{SkillExposure, SkillSelector};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteItem {
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub keywords: Vec<String>,
    pub icon: Option<char>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteAction {
    SendCommand(String),
    SwitchSession(ThreadId),
    SwitchModel {
        provider: String,
        model: String,
    },
    SetPolicyMode(PolicyMode),
    SetWebSearchMode(HostedWebSearchMode),
    SetSearchIndexEnabled(bool),
    SetSkillEnabled {
        selector: SkillSelector,
        enabled: bool,
    },
    SetSkillExposure {
        selector: SkillSelector,
        exposure: SkillExposure,
    },
    SelectRunner {
        destination_id: String,
        provider_id: String,
    },
    InsertComposerText(String),
    OpenPluginBrowser,
    OpenSkillsManager,
    ShowAutomationsStatus,
    /// Switch the active theme by id (basename of the `.css` file, no
    /// extension). The dispatcher reloads the stylesheet, restyles the next
    /// frame, and persists the choice to `~/.roder/state.toml`.
    SetTheme(String),
}

pub trait PaletteSource: Send + Sync + 'static {
    fn id(&self) -> &str;
    fn label(&self) -> &str;
    fn items(&self) -> Vec<PaletteItem>;
    fn execute(&self, item_id: &str) -> Option<PaletteAction>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteEntry {
    pub source_id: String,
    pub source_label: String,
    pub item: PaletteItem,
    pub action: PaletteAction,
}

pub struct StaticPaletteSource {
    id: String,
    label: String,
    entries: Vec<(PaletteItem, PaletteAction)>,
}

impl StaticPaletteSource {
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        entries: Vec<(PaletteItem, PaletteAction)>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            entries,
        }
    }

    pub fn entries(&self) -> Vec<PaletteEntry> {
        self.entries
            .iter()
            .map(|(item, action)| PaletteEntry {
                source_id: self.id.clone(),
                source_label: self.label.clone(),
                item: item.clone(),
                action: action.clone(),
            })
            .collect()
    }
}

impl PaletteSource for StaticPaletteSource {
    fn id(&self) -> &str {
        &self.id
    }

    fn label(&self) -> &str {
        &self.label
    }

    fn items(&self) -> Vec<PaletteItem> {
        self.entries.iter().map(|(item, _)| item.clone()).collect()
    }

    fn execute(&self, item_id: &str) -> Option<PaletteAction> {
        self.entries
            .iter()
            .find(|(item, _)| item.id == item_id)
            .map(|(_, action)| action.clone())
    }
}

pub fn collect_entries(sources: &[StaticPaletteSource]) -> Vec<PaletteEntry> {
    sources
        .iter()
        .flat_map(StaticPaletteSource::entries)
        .collect()
}

pub fn cycle_source_filter(current: Option<&str>, entries: &[PaletteEntry]) -> Option<String> {
    let mut source_ids = entries
        .iter()
        .map(|entry| entry.source_id.as_str())
        .collect::<Vec<_>>();
    source_ids.sort_unstable();
    source_ids.dedup();

    match current {
        None => source_ids.first().map(|id| (*id).to_string()),
        Some(active) => source_ids
            .iter()
            .position(|id| *id == active)
            .and_then(|index| source_ids.get(index + 1))
            .map(|id| (*id).to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_source_executes_known_items() {
        let source = StaticPaletteSource::new(
            "commands",
            "Commands",
            vec![(
                PaletteItem {
                    id: "review".to_string(),
                    title: "Review".to_string(),
                    subtitle: None,
                    keywords: vec!["code".to_string()],
                    icon: Some('/'),
                },
                PaletteAction::SendCommand("review".to_string()),
            )],
        );

        assert_eq!(
            source.execute("review"),
            Some(PaletteAction::SendCommand("review".to_string()))
        );
        assert_eq!(source.execute("missing"), None);
    }

    #[test]
    fn source_filter_cycles_through_sources_then_all() {
        let entries = vec![entry("commands"), entry("models"), entry("sessions")];
        assert_eq!(
            cycle_source_filter(None, &entries),
            Some("commands".to_string())
        );
        assert_eq!(
            cycle_source_filter(Some("commands"), &entries),
            Some("models".to_string())
        );
        assert_eq!(cycle_source_filter(Some("sessions"), &entries), None);
    }

    fn entry(source_id: &str) -> PaletteEntry {
        PaletteEntry {
            source_id: source_id.to_string(),
            source_label: source_id.to_string(),
            item: PaletteItem {
                id: source_id.to_string(),
                title: source_id.to_string(),
                subtitle: None,
                keywords: Vec::new(),
                icon: None,
            },
            action: PaletteAction::InsertComposerText(String::new()),
        }
    }
}
