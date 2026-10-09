//! Explicit runner or local macOS Cua desktop tools. No implicit fallback.
mod browser;
mod browser_specs;
mod capture;
mod executor;
mod local;
mod policy;
mod render;
mod specs;
mod transport;

pub use executor::CuaToolContributor;
pub use local::{LocalDesktopLease, LocalMacosTransport};
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
pub use roder_config::cua::{CuaBackend, CuaConfig};
use semver::Version;
pub use specs::cua_tool_specs;
use std::sync::Arc;
pub use transport::{CuaTarget, CuaTransport, DriverReply, RunnerCuaTransport};

pub const DRIVER_VERSION: &str = "0.34.0";

pub struct CuaExtension {
    config: CuaConfig,
    transport: Option<Arc<dyn CuaTransport>>,
}

impl CuaExtension {
    pub fn new(config: CuaConfig) -> Self {
        Self {
            config,
            transport: None,
        }
    }
    /// Install a custom transport while preserving Cua routing and policy checks.
    pub fn with_transport(config: CuaConfig, transport: Arc<dyn CuaTransport>) -> Self {
        Self {
            config,
            transport: Some(transport),
        }
    }
}

impl RoderExtension for CuaExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-cua".into(),
            name: "Cua Desktop Computer Use".into(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".into(),
            description: Some(
                "Observe and operate the thread's runner desktop or an explicitly selected local macOS desktop.".into(),
            ),
            provides: vec![
                ProvidedService::ToolProvider("cua".into()),
                ProvidedService::PolicyContributor("cua".into()),
            ],
            required_capabilities: vec![],
        }
    }
    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        self.config.validate()?;
        let contributor = match &self.transport {
            Some(transport) => {
                CuaToolContributor::with_transport(self.config.clone(), transport.clone())
            }
            None => CuaToolContributor::new(self.config.clone()),
        };
        registry.tool_contributor(Arc::new(contributor));
        registry.policy_contributor(Arc::new(policy::CuaPolicy));
        Ok(())
    }
}
