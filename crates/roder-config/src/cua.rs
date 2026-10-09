//! Explicit selection of a runner desktop or the local macOS desktop.
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CuaBackend {
    #[default]
    Runner,
    LocalMacos,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct CuaConfig {
    pub enabled: bool,
    pub backend: CuaBackend,
    /// Trusted launcher/client; defaults to the selected backend's program.
    pub program: Option<String>,
    /// Local app-owned daemon socket. Models cannot select or change it.
    pub socket_path: Option<String>,
    pub timeout_ms: u64,
    pub max_image_dimension: u32,
    /// Permit requests to attach an existing profile. The driver also needs its own launch-time grant.
    pub allow_existing_browser_profile: bool,
}

impl Default for CuaConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            backend: CuaBackend::Runner,
            program: None,
            socket_path: None,
            timeout_ms: 45_000,
            max_image_dimension: 1280,
            allow_existing_browser_profile: false,
        }
    }
}

impl CuaConfig {
    pub fn program(&self) -> &str {
        self.program.as_deref().unwrap_or(match self.backend {
            CuaBackend::Runner => "/opt/roder-cua/bin/cua-call",
            CuaBackend::LocalMacos => "/Applications/CuaDriver.app/Contents/MacOS/cua-driver",
        })
    }

    pub fn local_socket_path(&self) -> anyhow::Result<String> {
        if let Some(path) = &self.socket_path {
            return Ok(path.clone());
        }
        let home = std::env::var("HOME")?;
        anyhow::ensure!(
            home.starts_with('/'),
            "HOME must be absolute for the local Cua socket"
        );
        Ok(format!("{home}/Library/Caches/cua-driver/cua-driver.sock"))
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.program().starts_with('/') && !self.program().contains('\0'),
            "cua.program must be an absolute executable path"
        );
        match self.backend {
            CuaBackend::Runner => anyhow::ensure!(
                self.socket_path.is_none(),
                "cua.socket_path is only valid for backend = local-macos"
            ),
            CuaBackend::LocalMacos => {
                let path = self.local_socket_path()?;
                anyhow::ensure!(
                    path.starts_with('/') && !path.contains('\0') && path.len() < 104,
                    "cua.socket_path must be an absolute macOS Unix socket path shorter than 104 bytes"
                );
            }
        }
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
        assert!(!cua.allow_existing_browser_profile);
        let granted: crate::Config =
            toml::from_str("[cua]\nallow_existing_browser_profile=true\n").unwrap();
        assert!(granted.cua.unwrap().allow_existing_browser_profile);
        assert!(toml::from_str::<crate::Config>("[cua]\nsandbox = 'other'\n").is_err());
    }
    #[test]
    fn backend_defaults_and_socket_validation_are_explicit() {
        let runner = CuaConfig::default();
        assert_eq!(runner.backend, CuaBackend::Runner);
        assert_eq!(runner.program(), "/opt/roder-cua/bin/cua-call");
        let config: crate::Config =
            toml::from_str("[cua]\nenabled=true\nbackend='local-macos'\n").unwrap();
        let mut local = config.cua.unwrap();
        assert_eq!(
            local.program(),
            "/Applications/CuaDriver.app/Contents/MacOS/cua-driver"
        );
        assert!(
            local
                .local_socket_path()
                .unwrap()
                .ends_with("/Library/Caches/cua-driver/cua-driver.sock")
        );
        local.validate().unwrap();
        for path in [
            "relative.sock".into(),
            format!("/{}", "x".repeat(103)),
            "/tmp/\0.sock".into(),
        ] {
            local.socket_path = Some(path);
            assert!(local.validate().is_err());
        }
        local.socket_path = Some("/tmp/cua-owned.sock".into());
        local.validate().unwrap();
        local.backend = CuaBackend::Runner;
        assert!(local.validate().is_err());
        assert!(toml::from_str::<crate::Config>("[cua]\nbackend='automatic'\n").is_err());
    }
}
