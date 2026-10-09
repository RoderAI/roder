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

use crate::browser_loss::{CallFailure, announce_fresh_browser, lost_browser_error};
use crate::catalog::missing_remote_tools;
use crate::failed_action::mark_failed;
use crate::launch::{BrowserUseConfig, DEFAULT_PACKAGE, resolve_uvx, server_command};
use crate::observation;
use crate::select_guard::ShownSelects;
use crate::state_view::{GET_STATE_REMOTE, compact_result};

/// Produces the launch command; replaced in tests by a fake server.
pub type LaunchSpec = Arc<dyn Fn() -> anyhow::Result<McpStdioServerConfig> + Send + Sync + 'static>;

pub struct BrowserUseServer {
    launch: LaunchSpec,
    package: String,
    client: Mutex<Option<Arc<McpStdioClient>>>,
    threads: Mutex<HashMap<String, Arc<BrowserUseServer>>>,
    operation: Mutex<()>,
    cancelled: Arc<AtomicBool>,
    /// A browser was discarded (server exited, or an earlier call failed or
    /// was cancelled) and no result has said so yet.
    reset_unannounced: AtomicBool,
    /// The native selects of the last page state shown for this browser.
    shown_selects: ShownSelects,
    profile: std::sync::Mutex<Option<crate::profile::OwnedProfile>>,
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
            reset_unannounced: AtomicBool::new(false),
            shown_selects: ShownSelects::default(),
            profile: std::sync::Mutex::new(None),
        }
    }

    /// What this thread's browser last showed the model about its selects.
    pub(crate) fn shown_selects(&self) -> &ShownSelects {
        &self.shown_selects
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

    /// Keep an action and its fresh observation together, report first. Cancellation
    /// invalidates the owned server and stops its process tree before the next call can
    /// reuse it. So does any failure after the call reached the server (transport error,
    /// timeout, the server dying, a failed observation); its error then says the browser
    /// is gone, and the first result from the replacement browser says it is a fresh
    /// one. The exception is the server answering the action with a JSON-RPC error: the
    /// server is alive and turned the call down, so the browser stays and the error is
    /// returned as it is.
    pub(crate) async fn call_observed(
        &self,
        remote: &str,
        arguments: Value,
        timeout: Duration,
        observe: bool,
    ) -> anyhow::Result<Value> {
        let _operation = self.operation.lock().await;
        let client = self.client().await?;
        if remote == "browser_navigate"
            && let Some(ceiling) = client.config().env.get("BROWSER_USE_ALLOWED_DOMAINS")
        {
            let url = arguments["url"].as_str().unwrap_or_default();
            let url = reqwest::Url::parse(url)?;
            anyhow::ensure!(
                matches!(url.scheme(), "http" | "https")
                    && url.host_str().is_some_and(|host| ceiling
                        .split(',')
                        .any(|domain| domain.eq_ignore_ascii_case(host))),
                "Navigation blocked by RODER_BROWSER_USE_ALLOWED_DOMAINS"
            );
        }
        if remote == "retry_with_browser_use_agent"
            && let Some(ceiling) = client.config().env.get("BROWSER_USE_ALLOWED_DOMAINS")
            && let Some(requested) = arguments.get("allowed_domains").and_then(Value::as_array)
        {
            let allowed: Vec<_> = ceiling.split(',').collect();
            anyhow::ensure!(
                requested.iter().all(|domain| domain
                    .as_str()
                    .is_some_and(|domain| allowed.contains(&domain))),
                "allowed_domains must be a subset of RODER_BROWSER_USE_ALLOWED_DOMAINS"
            );
        }
        let mut pending = PendingCall {
            client: client.clone(),
            cancelled: self.cancelled.clone(),
            armed: true,
        };
        // Any failure from here on leaves `pending` armed, so the server and
        // its browser are stopped and the error must say so, unless the server
        // answered the action with a JSON-RPC error.
        let called = call_and_observe(
            &client,
            remote,
            arguments,
            timeout,
            observe,
            &self.shown_selects,
        )
        .await;
        match called {
            Ok(mut result) => {
                pending.armed = false;
                if self.reset_unannounced.swap(false, Ordering::SeqCst) {
                    announce_fresh_browser(&mut result);
                }
                Ok(result)
            }
            Err(CallFailure::Rejected(error)) => {
                // Nothing ran and nothing was stopped: the page the model was
                // shown, selects included, is still the page.
                pending.armed = false;
                Err(error)
            }
            Err(CallFailure::Lost(error)) => {
                // The browser the shown page belonged to is being stopped.
                self.shown_selects.replace(None);
                Err(lost_browser_error(error))
            }
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
        self.profile.lock().unwrap().take();
        self.shown_selects.replace(None);
    }

    async fn client(&self) -> anyhow::Result<Arc<McpStdioClient>> {
        let mut slot = self.client.lock().await;
        if let Some(client) = slot.as_ref() {
            if !client.has_exited() && !self.cancelled.swap(false, Ordering::SeqCst) {
                return Ok(client.clone());
            }
            client.shutdown().await;
            *slot = None;
            self.profile.lock().unwrap().take();
            self.shown_selects.replace(None);
            self.reset_unannounced.store(true, Ordering::SeqCst);
        }
        let mut spec = (self.launch)()?;
        let profile = crate::profile::OwnedProfile::configure(&mut spec)?;
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
        *self.profile.lock().unwrap() = Some(profile);
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

/// Runs one remote tool and, when asked, appends a fresh `browser_get_state`
/// of the same browser after the action's report. The state shown this way
/// replaces what `shown_selects` knows about the page.
async fn call_and_observe(
    client: &McpStdioClient,
    remote: &str,
    arguments: Value,
    timeout: Duration,
    observe: bool,
    shown_selects: &ShownSelects,
) -> Result<Value, CallFailure> {
    let mut result = client
        .call_tool_raw(remote, arguments, timeout)
        .await
        .map_err(CallFailure::of_action)?;
    mark_failed(remote, &mut result);
    if observe {
        // The action ran. A failure to read the page after it is not the
        // model's mistake, whatever the server answers, so it is a loss.
        let mut state = client
            .call_tool_raw(
                GET_STATE_REMOTE,
                serde_json::json!({"include_screenshot": true}),
                Duration::from_secs(30),
            )
            .await
            .map_err(CallFailure::Lost)?;
        mark_failed(GET_STATE_REMOTE, &mut state);
        // The same view `browser_use_get_state` shows, from its first element.
        shown_selects.replace(compact_result(&mut state, 0, &client.config().redact));
        observation::attach(&mut result, state).map_err(CallFailure::Lost)?;
    }
    Ok(result)
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
