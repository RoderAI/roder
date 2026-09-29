//! browser-use browser provider for Roder.
//!
//! Exposes the open-source browser-use local MCP server
//! (`uvx --from 'browser-use[cli]' browser-use --mcp`) as `browser_use_*`
//! tools. The server speaks MCP over stdio through `roder-ext-mcp`'s
//! [`roder_ext_mcp::McpStdioClient`], starts on the first tool call, gets
//! only an allowlisted environment plus the OpenAI/Anthropic keys Roder
//! already holds, and is stopped with its browser when Roder exits. A policy
//! contributor gates clicks, typing and the autonomous agent tool; page
//! content comes back labeled untrusted.
//!
//! The provider is opt-in: install [`BrowserUseExtension`] only when the user
//! enabled it (`[browser_use] enabled = true` or `roder --browser-use`).

mod catalog;
mod launch;
mod policy;
mod server;
mod tools;

use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use semver::Version;

pub use catalog::{AGENT_TOOL, BrowserUseToolDef, missing_remote_tools, pinned_tools, tool_defs};
pub use launch::{
    BrowserUseConfig, DEFAULT_PACKAGE, SERVER_NAME, resolve_uvx, server_command, server_env,
};
pub use policy::{BrowserUseActionClass, classify_tool};
pub use server::{BrowserUseServer, LaunchSpec};
pub use tools::{BrowserUseToolContributor, TOOL_PROVIDER_ID, UNTRUSTED_NOTE};

/// The browser-use provider: `browser_use_*` tools and their policy.
pub struct BrowserUseExtension {
    server: Arc<BrowserUseServer>,
    has_llm_key: bool,
}

impl BrowserUseExtension {
    pub fn new(config: BrowserUseConfig) -> Self {
        Self {
            server: Arc::new(BrowserUseServer::new(&config)),
            has_llm_key: config.has_llm_key(),
        }
    }

    /// Settings from the environment; see [`BrowserUseConfig::from_env`].
    pub fn from_env() -> Self {
        Self::new(BrowserUseConfig::from_env())
    }

    /// An extension bound to a prepared server (tests, embedders).
    pub fn with_server(server: Arc<BrowserUseServer>, has_llm_key: bool) -> Self {
        Self {
            server,
            has_llm_key,
        }
    }

    pub fn server(&self) -> &Arc<BrowserUseServer> {
        &self.server
    }
}

impl RoderExtension for BrowserUseExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-browser-use".into(),
            name: "browser-use Browser (MCP)".into(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".into(),
            description: Some(
                "Drive a separate browser through the browser-use local MCP server.".into(),
            ),
            provides: vec![
                ProvidedService::ToolProvider(TOOL_PROVIDER_ID.into()),
                ProvidedService::PolicyContributor("browser-use".into()),
            ],
            required_capabilities: vec![
                CapabilityRequest::new("network.web"),
                CapabilityRequest::new("process.uvx"),
            ],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.tool_contributor(Arc::new(BrowserUseToolContributor::new(
            self.server.clone(),
            self.has_llm_key,
        )));
        registry.policy_contributor(Arc::new(policy::BrowserUsePolicy));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installs_tools_and_policy() {
        let mut builder = ExtensionRegistryBuilder::new();
        builder
            .install(BrowserUseExtension::new(BrowserUseConfig::default()))
            .unwrap();
        let registry = builder.build().unwrap();
        let services = registry.provided_services();
        assert!(services.contains(&ProvidedService::ToolProvider("browser-use".into())));
        assert!(services.contains(&ProvidedService::PolicyContributor("browser-use".into())));
    }
}
