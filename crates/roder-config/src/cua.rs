//! Configuration for a Cua driver installed in a thread's remote desktop.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct CuaConfig {
    pub enabled: bool,
    /// Trusted runner-side launcher; never supplied by a model tool call.
    pub program: String,
    pub timeout_ms: u64,
    pub max_image_dimension: u32,
}

impl Default for CuaConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            program: "/opt/roder-cua/bin/cua-call".into(),
            timeout_ms: 45_000,
            max_image_dimension: 1280,
        }
    }
}

impl CuaConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.program.starts_with('/') && !self.program.contains('\0'),
            "cua.program must be an absolute runner-side executable path"
        );
        anyhow::ensure!(
            (1000..=120_000).contains(&self.timeout_ms),
            "cua.timeout_ms must be between 1000 and 120000"
        );
        anyhow::ensure!(
            self.max_image_dimension <= 4096,
            "cua.max_image_dimension must be 0 (native) or at most 4096"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn config_is_opt_in_and_rejects_unknown_routing() {
        let config: crate::Config = toml::from_str("[cua]\nenabled = true\n").unwrap();
        let cua = config.cua.unwrap();
        assert!(cua.enabled);
        cua.validate().unwrap();
        assert!(!CuaConfig::default().enabled);
        assert!(toml::from_str::<crate::Config>("[cua]\nsandbox = 'other'\n").is_err());
    }
}
