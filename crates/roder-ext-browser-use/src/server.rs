//! Lazily started, process-wide browser-use MCP server.
//!
//! Registering the tools does not start anything: the server (and, on its
//! first browser call, its browser) starts on the first `browser_use_*` call
//! and is reused by every thread of this Roder process. It is stopped when
//! the extension is dropped at Roder shutdown and, if Roder dies without
//! running destructors, by the parent guard in `roder-ext-mcp`.

use std::sync::Arc;
use std::time::Duration;

use roder_ext_mcp::{McpStdioClient, McpStdioServerConfig};
use serde_json::Value;
use tokio::sync::Mutex;

use crate::catalog::missing_remote_tools;
use crate::launch::{BrowserUseConfig, DEFAULT_PACKAGE, resolve_uvx, server_command};

/// Produces the launch command; replaced in tests by a fake server.
pub type LaunchSpec = Arc<dyn Fn() -> anyhow::Result<McpStdioServerConfig> + Send + Sync + 'static>;

pub struct BrowserUseServer {
    launch: LaunchSpec,
    package: String,
    client: Mutex<Option<Arc<McpStdioClient>>>,
}

impl BrowserUseServer {
    pub fn new(config: &BrowserUseConfig) -> Self {
        let launch_config = config.clone();
        Self::with_launch(
            config.package.clone(),
            Arc::new(move || {
                let uvx = resolve_uvx(&launch_config)?;
                Ok(server_command(&launch_config, uvx, std::env::vars()))
            }),
        )
    }

    pub fn with_launch(package: String, launch: LaunchSpec) -> Self {
        Self {
            launch,
            package,
            client: Mutex::new(None),
        }
    }

    /// Calls a remote tool, starting (or restarting after a crash) the server
    /// first when needed.
    pub async fn call(
        &self,
        remote: &str,
        arguments: Value,
        timeout: Duration,
    ) -> anyhow::Result<Value> {
        let client = self.client().await?;
        client.call_tool_raw(remote, arguments, timeout).await
    }

    /// The running client's redaction list (empty before the first start).
    pub async fn redactions(&self) -> Vec<String> {
        match self.client.lock().await.as_ref() {
            Some(client) => client.config().redact.clone(),
            None => Vec::new(),
        }
    }

    /// Stops the server and its browser, if running: first asks browser-use
    /// to close its browser sessions cleanly, then stops the process group.
    pub async fn shutdown(&self) {
        if let Some(client) = self.client.lock().await.take() {
            if !client.has_exited() {
                let _ = client
                    .call_tool_raw(
                        "browser_close_all",
                        serde_json::json!({}),
                        Duration::from_secs(5),
                    )
                    .await;
            }
            client.shutdown().await;
        }
    }

    async fn client(&self) -> anyhow::Result<Arc<McpStdioClient>> {
        let mut slot = self.client.lock().await;
        if let Some(client) = slot.as_ref() {
            if !client.has_exited() {
                return Ok(client.clone());
            }
            client.shutdown().await;
            *slot = None;
        }
        let spec = (self.launch)()?;
        let client = McpStdioClient::start(spec).await.map_err(|error| {
            anyhow::anyhow!(
                "browser-use: the MCP server failed to start ({error:#}). Check that \
                 `uvx --from '{}' browser-use --mcp` runs in a terminal, that Python >= 3.11 is \
                 available to uv, and that Chrome or Chromium is installed.",
                self.package
            )
        })?;
        let offered = client.list_tools(Duration::from_secs(30)).await?;
        let missing = missing_remote_tools(&offered);
        if !missing.is_empty() {
            client.shutdown().await;
            anyhow::bail!(
                "browser-use: the server from {} does not offer {}; Roder's browser-use tools \
                 match {DEFAULT_PACKAGE}. Set [browser_use] package to that release.",
                self.package,
                missing.join(", ")
            );
        }
        let client = Arc::new(client);
        *slot = Some(client.clone());
        Ok(client)
    }
}

impl Drop for BrowserUseServer {
    fn drop(&mut self) {
        // Dropping the last client handle stops the server's process group
        // (see `McpStdioClient`'s `Drop`).
        self.client.get_mut().take();
    }
}
