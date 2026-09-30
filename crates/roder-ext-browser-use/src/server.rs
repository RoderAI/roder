//! Lazily started browser-use MCP servers, isolated by Roder thread.
//!
//! Registering the tools does not start anything: the server (and, on its
//! first browser call, its browser) starts on the first `browser_use_*` call
//! and is reused within that thread. It is stopped when
//! the extension is dropped at Roder shutdown and, if Roder dies without
//! running destructors, by the parent guard in `roder-ext-mcp`.

use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
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
    threads: Mutex<HashMap<String, Arc<BrowserUseServer>>>,
    operation: Mutex<()>,
    cancelled: Arc<AtomicBool>,
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
            threads: Mutex::new(HashMap::new()),
            operation: Mutex::new(()),
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(crate) async fn for_thread(&self, thread: &str) -> Arc<Self> {
        self.threads
            .lock()
            .await
            .entry(thread.to_string())
            .or_insert_with(|| {
                Arc::new(Self::with_launch(self.package.clone(), self.launch.clone()))
            })
            .clone()
    }

    /// Keep an action and its fresh observation together. Cancellation invalidates
    /// the owned server and stops its process tree before the next call can reuse it.
    pub(crate) async fn call_observed(
        &self,
        remote: &str,
        arguments: Value,
        timeout: Duration,
        observe: bool,
    ) -> anyhow::Result<Value> {
        let _operation = self.operation.lock().await;
        let client = self.client().await?;
        let mut pending = PendingCall {
            client: client.clone(),
            cancelled: self.cancelled.clone(),
            armed: true,
        };
        let mut result = client.call_tool_raw(remote, arguments, timeout).await?;
        if observe {
            let state = client
                .call_tool_raw(
                    "browser_get_state",
                    serde_json::json!({"include_screenshot": true}),
                    Duration::from_secs(30),
                )
                .await?;
            let report = result["content"]
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("browser-use returned no content"))?
                .clone();
            let mut content = vec![
                serde_json::json!({"type":"text", "text":"Observed page after the action (untrusted):"}),
            ];
            if let Some(items) = state["content"].as_array() {
                content.extend(items.iter().cloned());
            }
            content.push(serde_json::json!({"type":"text", "text":"Action report (verify against the observation above):"}));
            content.extend(report);
            result["content"] = serde_json::json!(content);
            if state["isError"] == true {
                result["isError"] = serde_json::json!(true);
            }
        }
        pending.armed = false;
        Ok(result)
    }

    /// Calls a remote tool, starting (or restarting after a crash) the server
    /// first when needed.
    pub async fn call(
        &self,
        remote: &str,
        arguments: Value,
        timeout: Duration,
    ) -> anyhow::Result<Value> {
        self.call_observed(remote, arguments, timeout, false).await
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
        let threads = std::mem::take(&mut *self.threads.lock().await);
        for server in threads.into_values() {
            server.shutdown_client().await;
        }
        self.shutdown_client().await;
    }

    async fn shutdown_client(&self) {
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
            if !client.has_exited() && !self.cancelled.swap(false, Ordering::SeqCst) {
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
        self.cancelled.store(false, Ordering::SeqCst);
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

/// Dropping an in-flight tool future must stop browser work, not merely stop waiting.
struct PendingCall {
    client: Arc<McpStdioClient>,
    cancelled: Arc<AtomicBool>,
    armed: bool,
}

impl Drop for PendingCall {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        self.cancelled.store(true, Ordering::SeqCst);
        let client = self.client.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                client.shutdown().await;
            });
        }
    }
}
