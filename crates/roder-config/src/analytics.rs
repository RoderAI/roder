//! Local usage-analytics configuration (roadmap phase 73).

use serde::{Deserialize, Serialize};

/// `[analytics]` config block. Analytics are local-only; nothing here
/// enables any remote upload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AnalyticsConfig {
    /// Record runtime usage into the local SQLite store (default: true).
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Database path override; default `<data-dir>/analytics/usage.sqlite3`.
    #[serde(default)]
    pub store: Option<String>,
    /// How workspace paths are recorded: `full_path` (default), `hashed`,
    /// or `basename_only`.
    #[serde(default = "default_workspace_labels")]
    pub workspace_labels: String,
    /// Days of raw rows to keep; `0` (default) keeps everything.
    #[serde(default)]
    pub retention_days: u32,
}

impl Default for AnalyticsConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            store: None,
            workspace_labels: default_workspace_labels(),
            retention_days: 0,
        }
    }
}

fn default_enabled() -> bool {
    true
}

fn default_workspace_labels() -> String {
    "full_path".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analytics_config_defaults_are_local_and_enabled() {
        let config: AnalyticsConfig = toml::from_str("").unwrap();
        assert!(config.enabled);
        assert_eq!(config.workspace_labels, "full_path");
        assert_eq!(config.store, None);
        assert_eq!(config.retention_days, 0);

        let config: AnalyticsConfig = toml::from_str(
            r#"
            enabled = false
            workspace_labels = "basename_only"
            store = "/tmp/custom.sqlite3"
            retention_days = 90
            "#,
        )
        .unwrap();
        assert!(!config.enabled);
        assert_eq!(config.workspace_labels, "basename_only");
    }
}
