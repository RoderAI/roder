//! Cua Driver tools bound to the caller's remote runner. No local fallback.
mod capture;
mod executor;
mod policy;
mod specs;
mod transport;

pub use executor::CuaToolContributor;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
pub use roder_config::cua::CuaConfig;
use semver::Version;
pub use specs::cua_tool_specs;
use std::sync::Arc;
pub use transport::{CuaTransport, DriverReply, RunnerCuaTransport};

pub const DRIVER_VERSION: &str = "0.34.0";

pub struct CuaExtension {
    config: CuaConfig,
}

impl CuaExtension {
    pub fn new(config: CuaConfig) -> Self {
        Self { config }
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
                "Observe and operate a native Linux desktop in the thread's remote runner.".into(),
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
        registry.tool_contributor(Arc::new(CuaToolContributor::new(self.config.clone())));
        registry.policy_contributor(Arc::new(policy::CuaPolicy));
        Ok(())
    }
}
