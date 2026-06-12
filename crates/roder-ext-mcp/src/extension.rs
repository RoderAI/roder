use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use semver::Version;

use crate::client::{McpHttpClient, McpToolDescriptor};
use crate::config::McpServerConfig;
use crate::tool::{McpRemoteTool, McpToolContributor};

/// MCP client extension: discovers tools from the configured servers at
/// construction time and exposes them as agent tools.
pub struct McpToolsExtension {
    tools: Vec<Arc<McpRemoteTool>>,
}

impl McpToolsExtension {
    /// Connects to every configured server and lists its tools. Servers that
    /// fail discovery are skipped with a warning on stderr so one offline
    /// MCP server does not take down the whole distribution.
    pub fn discover(configs: Vec<McpServerConfig>) -> anyhow::Result<Self> {
        let discovered = discover_blocking(configs)?;
        Ok(Self::from_discovered(discovered))
    }

    /// Like [`Self::discover`] but fails if any configured server is
    /// unreachable. Use for servers the distribution cannot run without.
    pub fn discover_required(configs: Vec<McpServerConfig>) -> anyhow::Result<Self> {
        let discovered = discover_blocking(configs.clone())?;
        for config in &configs {
            if !discovered
                .iter()
                .any(|(client, _)| client.config().name == config.name)
            {
                anyhow::bail!("required MCP server {} failed discovery", config.name);
            }
        }
        Ok(Self::from_discovered(discovered))
    }

    /// Async variant for hosts that are already inside a Tokio runtime.
    pub async fn discover_async(configs: Vec<McpServerConfig>) -> anyhow::Result<Self> {
        Ok(Self::from_discovered(discover_servers(configs).await))
    }

    fn from_discovered(discovered: Vec<(McpHttpClient, Vec<McpToolDescriptor>)>) -> Self {
        let mut tools = Vec::new();
        for (client, descriptors) in discovered {
            for descriptor in descriptors {
                tools.push(Arc::new(McpRemoteTool::new(client.clone(), descriptor)));
            }
        }
        Self { tools }
    }

    pub fn tool_count(&self) -> usize {
        self.tools.len()
    }
}

impl RoderExtension for McpToolsExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-mcp".to_string(),
            name: "MCP Tools".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some(
                "Exposes tools from MCP servers (streamable HTTP) to the agent".to_string(),
            ),
            provides: vec![ProvidedService::ToolProvider(
                crate::tool::MCP_TOOL_PROVIDER_ID.to_string(),
            )],
            required_capabilities: vec![CapabilityRequest::new("network.mcp")],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.tool_contributor(Arc::new(McpToolContributor::new(self.tools.clone())));
        Ok(())
    }
}

/// Runs discovery on a dedicated thread with its own Tokio runtime so it is
/// safe to call from both sync `main` functions and async contexts.
fn discover_blocking(
    configs: Vec<McpServerConfig>,
) -> anyhow::Result<Vec<(McpHttpClient, Vec<McpToolDescriptor>)>> {
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        Ok(runtime.block_on(discover_servers(configs)))
    })
    .join()
    .map_err(|_| anyhow::anyhow!("MCP discovery thread panicked"))?
}

async fn discover_servers(
    configs: Vec<McpServerConfig>,
) -> Vec<(McpHttpClient, Vec<McpToolDescriptor>)> {
    let mut discovered = Vec::new();
    for config in configs {
        let name = config.name.clone();
        let client = match McpHttpClient::new(config) {
            Ok(client) => client,
            Err(error) => {
                eprintln!("warning: MCP server {name}: failed to build client: {error:#}");
                continue;
            }
        };
        match client.list_tools().await {
            Ok(tools) => discovered.push((client, tools)),
            Err(error) => {
                eprintln!("warning: MCP server {name}: tool discovery failed: {error:#}");
            }
        }
    }
    discovered
}

#[cfg(test)]
mod tests {
    use roder_api::extension::{ExtensionRegistryBuilder, ProvidedService, RoderExtension};

    use super::*;

    #[test]
    fn manifest_declares_mcp_tool_provider() {
        let extension = McpToolsExtension::from_discovered(Vec::new());
        let manifest = extension.manifest();

        assert_eq!(manifest.id, "roder-ext-mcp");
        assert_eq!(
            manifest.provides,
            vec![ProvidedService::ToolProvider("mcp-tools".to_string())]
        );
    }

    #[test]
    fn installs_tool_contributor() {
        let extension = McpToolsExtension::from_discovered(Vec::new());
        let mut builder = ExtensionRegistryBuilder::new();

        extension.install(&mut builder).unwrap();
        let registry = builder.build().unwrap();

        assert_eq!(registry.tools.len(), 1);
        assert_eq!(registry.tools[0].id(), "mcp-tools");
    }
}
