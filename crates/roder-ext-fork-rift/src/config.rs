//! Rift adapter configuration.

use std::path::PathBuf;

/// Environment override for the Rift binary path (also used by the opt-in
/// live tests as `RIFT_BIN`).
pub const RIFT_BIN_ENV: &str = "RODER_RIFT_BIN";

#[derive(Debug, Clone)]
pub struct RiftConfig {
    /// Path to the `rift` binary; default resolves `rift` from `PATH` or
    /// the `RODER_RIFT_BIN` env override.
    pub rift_bin: PathBuf,
    /// Base directory for created snapshot workspaces. Defaults to
    /// `<source>/.roder/rift-forks`.
    pub base_dir: Option<PathBuf>,
}

impl Default for RiftConfig {
    fn default() -> Self {
        Self {
            rift_bin: std::env::var_os(RIFT_BIN_ENV)
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("rift")),
            base_dir: None,
        }
    }
}
