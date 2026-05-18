use std::str::FromStr;

use roder_tui::{
    TuiAppConfig,
    keymap::{Action, KeyBindingOverride, Keymap},
};

pub(crate) fn resolve_tui_app_config(
    cfg: &roder_config::Config,
    registry: &roder_api::extension::ExtensionRegistry,
) -> TuiAppConfig {
    let tui = cfg.tui.clone().unwrap_or_default();
    TuiAppConfig {
        status_segments: registry.status_segments.clone(),
        disabled_status_segments: tui.status.disabled_segments.into_iter().collect(),
        palette_sources: registry.palette_sources.clone(),
        disabled_palette_sources: tui.palette.disabled_sources.into_iter().collect(),
        diff_enabled: tui.diff.enabled,
        keymap: resolve_keymap(tui.keymap),
    }
}

fn resolve_keymap(bindings: std::collections::HashMap<String, Vec<String>>) -> Keymap {
    let overrides = bindings.into_iter().filter_map(|(action, keys)| {
        Some(KeyBindingOverride {
            action: Action::from_str(&action).ok()?,
            keys,
        })
    });
    Keymap::default().with_overrides(overrides)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tui_integration_config_maps_to_tui_app_config() {
        let mut builder = roder_api::extension::ExtensionRegistryBuilder::new();
        builder.status_segment(roder_api::tui_status::StatusSegment::new(
            "mode",
            100,
            8,
            |_| roder_api::tui_status::StatusCell {
                text: "mode:test".to_string(),
                style: roder_api::tui_status::StatusStyle::Accent,
                tooltip: None,
            },
        ));
        builder.palette_source(roder_api::tui_status::PaletteSourceDescriptor {
            id: "commands".to_string(),
            label: "Commands".to_string(),
            priority: 100,
        });
        builder.palette_source(roder_api::tui_status::PaletteSourceDescriptor {
            id: "agents".to_string(),
            label: "Agents".to_string(),
            priority: 80,
        });
        let registry = builder.build().unwrap();
        let cfg = roder_config::Config {
            tui: Some(roder_config::TuiConfig {
                status: roder_config::TuiStatusConfig {
                    disabled_segments: vec!["mcp".to_string()],
                },
                palette: roder_config::TuiPaletteConfig {
                    disabled_sources: vec!["agents".to_string()],
                },
                diff: roder_config::TuiDiffConfig { enabled: false },
                keymap: std::collections::HashMap::from([(
                    "palette/open".to_string(),
                    vec!["ctrl+k".to_string()],
                )]),
            }),
            ..roder_config::Config::default()
        };

        let tui = resolve_tui_app_config(&cfg, &registry);

        assert_eq!(tui.status_segments.len(), 1);
        assert!(tui.disabled_status_segments.contains("mcp"));
        assert!(tui.disabled_palette_sources.contains("agents"));
        assert!(!tui.diff_enabled);
        assert_eq!(
            tui.keymap
                .bindings_for(roder_tui::keymap::Action::OpenPalette)[0]
                .key,
            "ctrl+k"
        );
    }
}
