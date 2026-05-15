use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TuiConfig {
    #[serde(default)]
    pub status: TuiStatusConfig,
    #[serde(default)]
    pub palette: TuiPaletteConfig,
    #[serde(default)]
    pub diff: TuiDiffConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TuiStatusConfig {
    #[serde(default)]
    pub disabled_segments: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TuiPaletteConfig {
    #[serde(default)]
    pub disabled_sources: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TuiDiffConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Default for TuiDiffConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use crate::Config;

    #[test]
    fn tui_integration_deserializes_tui_config() {
        let config: Config = toml::from_str(
            r#"
            [tui.status]
            disabled_segments = ["mcp", "usage"]

            [tui.palette]
            disabled_sources = ["agents"]

            [tui.diff]
            enabled = false
            "#,
        )
        .unwrap();

        let tui = config.tui.unwrap();
        assert_eq!(
            tui.status.disabled_segments,
            ["mcp".to_string(), "usage".to_string()]
        );
        assert_eq!(tui.palette.disabled_sources, ["agents".to_string()]);
        assert!(!tui.diff.enabled);
    }
}
