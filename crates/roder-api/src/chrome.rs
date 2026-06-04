//! Shared Chrome browser-bridge contract.
//!
//! This module is the single source of truth that ties together the four layers
//! of the Roder Chrome integration:
//!
//! * the Manifest V3 browser extension (`/Users/pz/w/roder-web-extention`), which
//!   speaks the JSON bridge envelope described below over the remote WebSocket
//!   app-server;
//! * the remote WebSocket transport in `roder-app-server` (`remote.rs`), which
//!   registers each connected extension with the [`ChromeBridge`] and forwards
//!   commands to it;
//! * the `chrome/*` app-server methods, which call [`ChromeController::dispatch`];
//! * the model-facing `chrome_*` tools in `roder-ext-chrome`, which are generic
//!   over an injected [`ChromeController`] so they can be unit-tested against a
//!   fake bridge.
//!
//! The single live [`ChromeBridge`] instance is reachable from every layer via
//! [`bridge`], a process-global singleton (there is exactly one browser bridge
//! per Roder process). Keeping the bridge here in `roder-api` lets both
//! `roder-app-server` and `roder-ext-chrome` share it without `roder-ext-chrome`
//! depending on `roder-core`.
//!
//! # Wire envelope
//!
//! * Roder → extension (command): `{ "type": "<command>", "id": "<corr>", ...params }`
//! * extension → Roder (command result):
//!   `{ "type": "command/result", "id": "<corr>", "ok": bool, "result"?: any, "error"?: string }`
//! * extension → Roder (unsolicited event):
//!   `{ "type": "hello"|"state"|"tabs/list"|"tab/updated"|"page/snapshot"|"activity"|"chat"|"output"|"debug/console"|"debug/network", ... }`
//!
//! Browser page content, DOM text, console output and network metadata are
//! **untrusted input** and are tagged with `untrusted: true` so model prompts do
//! not treat page content as user or system instructions.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};

/// How aggressively Roder may act in the browser on the user's behalf.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ChromePermissionMode {
    /// Chat, tab status and connection state only.
    Observe,
    /// Inspect actions can run when the site permits; privileged actions queue
    /// for approval.
    #[default]
    Assist,
    /// Enabled actions execute inside the approved plan and site scope;
    /// protected actions still require explicit approval.
    Control,
}

impl ChromePermissionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            ChromePermissionMode::Observe => "observe",
            ChromePermissionMode::Assist => "assist",
            ChromePermissionMode::Control => "control",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "observe" => Some(ChromePermissionMode::Observe),
            "assist" => Some(ChromePermissionMode::Assist),
            "control" => Some(ChromePermissionMode::Control),
            _ => None,
        }
    }
}

/// A browser the extension can drive (Chrome is P0, Edge is P1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChromeBrowser {
    pub id: String,
    pub name: String,
    /// `chrome` or `edge`.
    pub kind: String,
}

/// A tab visible to the extension.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChromeTab {
    pub id: i64,
    #[serde(default)]
    pub window_id: Option<i64>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub fav_icon_url: Option<String>,
    #[serde(default)]
    pub active: bool,
}

/// Per-origin site permission record stored by the extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ChromeSitePermission {
    pub origin: String,
    #[serde(default)]
    pub inspect: bool,
    #[serde(default)]
    pub interact: bool,
    #[serde(default)]
    pub eval: bool,
    #[serde(default)]
    pub debugger: bool,
    #[serde(default)]
    pub download: bool,
    #[serde(default)]
    pub upload: bool,
    #[serde(default)]
    pub recording: bool,
    #[serde(default)]
    pub schedule: bool,
    #[serde(default)]
    pub always_allow: bool,
}

/// Snapshot of the connection, mirrored to TUI/CLI/SDK clients via `chrome/status`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ChromeStatus {
    /// At least one extension is connected.
    pub connected: bool,
    /// Number of connected extension clients.
    pub client_count: usize,
    /// Whether Chrome tools are enabled for the session.
    pub enabled: bool,
    /// Capabilities advertised by the connected extension's `hello`.
    pub capabilities: Vec<String>,
    pub mode: ChromePermissionMode,
    #[serde(default)]
    pub active_tab: Option<ChromeTab>,
    #[serde(default)]
    pub browser: Option<ChromeBrowser>,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub remote_addr: Option<String>,
}

/// A command Roder asks the extension to run. `kind` is the wire `type`
/// (e.g. `"page/snapshot"`); `params` is merged into the outgoing frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChromeCommand {
    pub kind: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

impl ChromeCommand {
    pub fn new(kind: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            params: serde_json::Value::Null,
        }
    }

    pub fn with_params(kind: impl Into<String>, params: serde_json::Value) -> Self {
        Self {
            kind: kind.into(),
            params,
        }
    }
}

/// Failure surface for browser commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChromeError {
    /// No extension is connected.
    NotConnected,
    /// Chrome tools are not enabled for this session.
    Disabled,
    /// The command was rejected by the user or by site/mode policy.
    Rejected(String),
    /// The extension did not respond within the dispatch deadline.
    Timeout,
    /// The connection dropped before a result arrived.
    Disconnected,
    /// The extension returned an error result.
    Remote(String),
}

impl std::fmt::Display for ChromeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChromeError::NotConnected => write!(f, "no Chrome extension is connected"),
            ChromeError::Disabled => write!(f, "Chrome tools are not enabled for this session"),
            ChromeError::Rejected(reason) => write!(f, "browser action rejected: {reason}"),
            ChromeError::Timeout => write!(f, "Chrome extension did not respond in time"),
            ChromeError::Disconnected => {
                write!(f, "Chrome extension disconnected before responding")
            }
            ChromeError::Remote(message) => write!(f, "Chrome extension error: {message}"),
        }
    }
}

impl std::error::Error for ChromeError {}

/// Default time a `dispatch` will wait for the extension to answer.
pub const CHROME_DISPATCH_TIMEOUT: Duration = Duration::from_secs(30);

/// Abstraction over the live browser bridge so model tools can be tested against
/// a fake. Implemented by [`ChromeBridge`].
#[async_trait::async_trait]
pub trait ChromeController: Send + Sync + 'static {
    /// Current connection status.
    fn status(&self) -> ChromeStatus;

    /// Whether Chrome tools are enabled for the session.
    fn is_enabled(&self) -> bool {
        self.status().enabled
    }

    /// Enable/disable Chrome tools for the session.
    fn set_enabled(&self, enabled: bool);

    /// Set the permission mode.
    fn set_mode(&self, mode: ChromePermissionMode);

    /// Forward a command to the connected extension and await its result.
    async fn dispatch(&self, command: ChromeCommand) -> Result<serde_json::Value, ChromeError>;
}

/// Handle returned to the transport when an extension connects: the transport
/// drains `commands` and writes each frame to the socket.
pub struct ChromeClientRegistration {
    pub client_id: u64,
    pub commands: mpsc::UnboundedReceiver<serde_json::Value>,
}

struct ClientHandle {
    commands: mpsc::UnboundedSender<serde_json::Value>,
    capabilities: Vec<String>,
    remote_addr: Option<String>,
    active_tab: Option<ChromeTab>,
    browser: Option<ChromeBrowser>,
}

#[derive(Default)]
struct BridgeState {
    clients: HashMap<u64, ClientHandle>,
    pending: HashMap<String, oneshot::Sender<Result<serde_json::Value, String>>>,
    enabled: bool,
    mode: ChromePermissionMode,
    last_error: Option<String>,
}

/// The live connection registry. One instance per process (see [`bridge`]).
///
/// Transport integration (in `roder-app-server::remote`), on each connection:
/// ```ignore
/// // on first `hello` frame:
/// let reg = bridge().register_client(Some(remote_addr.clone()), &frame);
/// // spawn a task draining reg.commands -> websocket as Message::Text(JSON)
/// // for every inbound bridge frame: bridge().ingest_frame(Some(reg.client_id), frame);
/// // on disconnect: bridge().unregister_client(reg.client_id);
/// ```
pub struct ChromeBridge {
    state: Mutex<BridgeState>,
    next_client_id: AtomicU64,
    next_corr_id: AtomicU64,
    dispatch_timeout: Duration,
}

impl Default for ChromeBridge {
    fn default() -> Self {
        Self {
            state: Mutex::new(BridgeState::default()),
            next_client_id: AtomicU64::new(1),
            next_corr_id: AtomicU64::new(1),
            dispatch_timeout: CHROME_DISPATCH_TIMEOUT,
        }
    }
}

impl ChromeBridge {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    fn with_timeout(timeout: Duration) -> Self {
        Self {
            dispatch_timeout: timeout,
            ..Self::default()
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BridgeState> {
        self.state.lock().expect("chrome bridge mutex poisoned")
    }

    /// Register a newly-connected extension. Returns the receiver the transport
    /// must drain to deliver commands to that extension.
    pub fn register_client(
        &self,
        remote_addr: Option<String>,
        hello: &serde_json::Value,
    ) -> ChromeClientRegistration {
        let client_id = self.next_client_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::unbounded_channel();
        let capabilities = string_array(hello.get("capabilities"));
        let mut state = self.lock();
        state.clients.insert(
            client_id,
            ClientHandle {
                commands: tx,
                capabilities,
                remote_addr,
                active_tab: None,
                browser: None,
            },
        );
        state.last_error = None;
        ChromeClientRegistration {
            client_id,
            commands: rx,
        }
    }

    /// Remove a disconnected extension and fail any of its pending commands.
    pub fn unregister_client(&self, client_id: u64) {
        let mut state = self.lock();
        state.clients.remove(&client_id);
        if state.clients.is_empty() {
            // Fail in-flight commands; the sockets are gone.
            for (_, tx) in state.pending.drain() {
                let _ = tx.send(Err("extension disconnected".to_string()));
            }
        }
    }

    /// Route an inbound bridge frame from the extension: resolve a pending
    /// command result or fold an unsolicited event into bridge state.
    pub fn ingest_frame(&self, client_id: Option<u64>, frame: serde_json::Value) {
        let kind = frame.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match kind {
            "command/result" => {
                let Some(id) = frame.get("id").and_then(|v| v.as_str()) else {
                    return;
                };
                let sender = {
                    let mut state = self.lock();
                    state.pending.remove(id)
                };
                if let Some(sender) = sender {
                    let ok = frame.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
                    let payload = if ok {
                        Ok(frame.get("result").cloned().unwrap_or(serde_json::Value::Null))
                    } else {
                        Err(frame
                            .get("error")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown extension error")
                            .to_string())
                    };
                    let _ = sender.send(payload);
                }
            }
            "hello" => {
                let caps = string_array(frame.get("capabilities"));
                let mut state = self.lock();
                if let Some(id) = client_id
                    && let Some(handle) = state.clients.get_mut(&id)
                {
                    handle.capabilities = caps;
                }
            }
            "state" => {
                let tab = frame
                    .get("state")
                    .and_then(|s| s.get("activeTab"))
                    .and_then(|t| serde_json::from_value::<ChromeTab>(t.clone()).ok());
                let mut state = self.lock();
                if let Some(id) = client_id
                    && let Some(handle) = state.clients.get_mut(&id)
                {
                    handle.active_tab = tab;
                }
            }
            "tab/updated" => {
                let tab = frame
                    .get("tab")
                    .and_then(|t| serde_json::from_value::<ChromeTab>(t.clone()).ok());
                let mut state = self.lock();
                if let Some(id) = client_id
                    && let Some(handle) = state.clients.get_mut(&id)
                {
                    handle.active_tab = tab;
                }
            }
            _ => {}
        }
    }

    fn next_correlation_id(&self) -> String {
        format!("rc-{}", self.next_corr_id.fetch_add(1, Ordering::SeqCst))
    }
}

#[async_trait::async_trait]
impl ChromeController for ChromeBridge {
    fn status(&self) -> ChromeStatus {
        let state = self.lock();
        let primary = state.clients.values().next();
        ChromeStatus {
            connected: !state.clients.is_empty(),
            client_count: state.clients.len(),
            enabled: state.enabled,
            capabilities: primary.map(|c| c.capabilities.clone()).unwrap_or_default(),
            mode: state.mode,
            active_tab: primary.and_then(|c| c.active_tab.clone()),
            browser: primary.and_then(|c| c.browser.clone()),
            last_error: state.last_error.clone(),
            remote_addr: primary.and_then(|c| c.remote_addr.clone()),
        }
    }

    fn set_enabled(&self, enabled: bool) {
        self.lock().enabled = enabled;
    }

    fn set_mode(&self, mode: ChromePermissionMode) {
        self.lock().mode = mode;
    }

    async fn dispatch(&self, command: ChromeCommand) -> Result<serde_json::Value, ChromeError> {
        let corr = self.next_correlation_id();
        let (res_tx, res_rx) = oneshot::channel();

        // Build the outgoing frame `{ type, id, ...params }` and enqueue it on the
        // first connected client. Hold the lock only for the send.
        {
            let mut state = self.lock();
            if !state.enabled {
                return Err(ChromeError::Disabled);
            }
            let Some(handle) = state.clients.values().next() else {
                return Err(ChromeError::NotConnected);
            };
            let mut frame = serde_json::Map::new();
            frame.insert("type".to_string(), serde_json::Value::String(command.kind));
            frame.insert("id".to_string(), serde_json::Value::String(corr.clone()));
            if let serde_json::Value::Object(params) = command.params {
                for (key, value) in params {
                    if key != "type" && key != "id" {
                        frame.insert(key, value);
                    }
                }
            }
            if handle
                .commands
                .send(serde_json::Value::Object(frame))
                .is_err()
            {
                return Err(ChromeError::Disconnected);
            }
            state.pending.insert(corr.clone(), res_tx);
        }

        match tokio::time::timeout(self.dispatch_timeout, res_rx).await {
            Ok(Ok(Ok(result))) => Ok(result),
            Ok(Ok(Err(message))) => Err(ChromeError::Remote(message)),
            Ok(Err(_)) => Err(ChromeError::Disconnected),
            Err(_) => {
                self.lock().pending.remove(&corr);
                Err(ChromeError::Timeout)
            }
        }
    }
}

/// The process-global browser bridge. Every layer shares this instance.
pub fn bridge() -> Arc<ChromeBridge> {
    static BRIDGE: OnceLock<Arc<ChromeBridge>> = OnceLock::new();
    BRIDGE.get_or_init(|| Arc::new(ChromeBridge::new())).clone()
}

fn string_array(value: Option<&serde_json::Value>) -> Vec<String> {
    value
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn status_reports_disconnected_by_default() {
        let bridge = ChromeBridge::new();
        let status = bridge.status();
        assert!(!status.connected);
        assert_eq!(status.client_count, 0);
        assert_eq!(status.mode, ChromePermissionMode::Assist);
    }

    #[tokio::test]
    async fn dispatch_requires_enabled() {
        let bridge = ChromeBridge::new();
        let reg = bridge.register_client(None, &json!({ "capabilities": ["chat"] }));
        drop(reg.commands);
        let err = bridge
            .dispatch(ChromeCommand::new("tabs/list"))
            .await
            .unwrap_err();
        assert_eq!(err, ChromeError::Disabled);
    }

    #[tokio::test]
    async fn dispatch_round_trips_command_and_result() {
        let bridge = Arc::new(ChromeBridge::new());
        bridge.set_enabled(true);
        let mut reg = bridge.register_client(
            Some("127.0.0.1:9".to_string()),
            &json!({ "capabilities": ["tabs.list"] }),
        );

        // Simulated extension: read the command frame, echo a result.
        let echo = bridge.clone();
        let handle = tokio::spawn(async move {
            let frame = reg.commands.recv().await.expect("command frame");
            assert_eq!(frame["type"], "page/snapshot");
            assert_eq!(frame["tabId"], 7);
            let id = frame["id"].as_str().unwrap().to_string();
            echo.ingest_frame(
                Some(reg.client_id),
                json!({ "type": "command/result", "id": id, "ok": true, "result": { "title": "Example" } }),
            );
        });

        let result = bridge
            .dispatch(ChromeCommand::with_params(
                "page/snapshot",
                json!({ "tabId": 7 }),
            ))
            .await
            .expect("dispatch ok");
        assert_eq!(result["title"], "Example");
        handle.await.unwrap();

        let status = bridge.status();
        assert!(status.connected);
        assert_eq!(status.capabilities, vec!["tabs.list".to_string()]);
    }

    #[tokio::test]
    async fn dispatch_times_out_without_response() {
        let bridge = ChromeBridge::with_timeout(Duration::from_millis(20));
        bridge.set_enabled(true);
        let reg = bridge.register_client(None, &json!({}));
        // Keep the receiver alive but never answer.
        let _keep = reg.commands;
        let err = bridge
            .dispatch(ChromeCommand::new("tabs/list"))
            .await
            .unwrap_err();
        assert_eq!(err, ChromeError::Timeout);
    }

    #[tokio::test]
    async fn not_connected_when_no_clients() {
        let bridge = ChromeBridge::new();
        bridge.set_enabled(true);
        let err = bridge
            .dispatch(ChromeCommand::new("tabs/list"))
            .await
            .unwrap_err();
        assert_eq!(err, ChromeError::NotConnected);
    }
}
