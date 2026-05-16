use std::collections::BTreeSet;

use roder_api::tui_status::{PaletteSourceDescriptor, StatusSegment};

use crate::status_line::built_in_status_segments;

#[derive(Clone)]
pub struct TuiAppConfig {
    pub status_segments: Vec<StatusSegment>,
    pub disabled_status_segments: BTreeSet<String>,
    pub palette_sources: Vec<PaletteSourceDescriptor>,
    pub disabled_palette_sources: BTreeSet<String>,
    pub diff_enabled: bool,
}

impl Default for TuiAppConfig {
    fn default() -> Self {
        Self {
            status_segments: built_in_status_segments(),
            disabled_status_segments: BTreeSet::new(),
            palette_sources: built_in_palette_sources(),
            disabled_palette_sources: BTreeSet::new(),
            diff_enabled: true,
        }
    }
}

impl TuiAppConfig {
    pub fn enabled_palette_source_ids(&self) -> BTreeSet<String> {
        self.palette_sources
            .iter()
            .map(|source| source.id.clone())
            .filter(|id| !self.disabled_palette_sources.contains(id))
            .collect()
    }
}

pub fn built_in_palette_sources() -> Vec<PaletteSourceDescriptor> {
    [
        ("commands", "Commands", 100),
        ("sessions", "Sessions", 90),
        ("agents", "Agents", 80),
        ("models", "Models", 70),
        ("modes", "Modes", 60),
        ("settings", "Settings", 50),
    ]
    .into_iter()
    .map(|(id, label, priority)| PaletteSourceDescriptor {
        id: id.to_string(),
        label: label.to_string(),
        priority,
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tui_integration_config_filters_disabled_palette_sources() {
        let config = TuiAppConfig {
            disabled_palette_sources: ["agents".to_string(), "models".to_string()]
                .into_iter()
                .collect(),
            ..TuiAppConfig::default()
        };

        let ids = config.enabled_palette_source_ids();
        assert!(ids.contains("commands"));
        assert!(ids.contains("sessions"));
        assert!(ids.contains("modes"));
        assert!(ids.contains("settings"));
        assert!(!ids.contains("agents"));
        assert!(!ids.contains("models"));
    }
}
