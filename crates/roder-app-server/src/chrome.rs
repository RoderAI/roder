//! `chrome/*` app-server methods bridging JSON-RPC clients to the connected
//! browser extension.
//!
//! Every handler talks to the process-global browser bridge via
//! [`roder_api::chrome::bridge`]; the transport that registers extensions lives
//! in [`crate::remote`]. Dispatch results (page snapshots, console output,
//! network metadata, permission records) originate from **untrusted** browser
//! context and are returned to the caller verbatim as opaque JSON — handlers do
//! not reinterpret or reformat them as instructions.

use roder_api::chrome::{
    ChromeCommand, ChromeController, ChromeError, ChromePermissionMode, bridge,
};
use roder_protocol::{
    ChromeDebugReadParams, ChromeEnableParams, ChromeNavigateParams, ChromePageActionParams,
    ChromePageSnapshotParams, ChromePermissionsListParams, ChromePermissionsUpdateParams,
    ChromeSetModeParams, ChromeTabActivateParams, JsonRpcError,
};

use crate::server::AppServer;

/// Wire actions accepted by `chrome/page/action`, mapped to `page/<action>`.
const PAGE_ACTIONS: &[&str] = &[
    "click",
    "type",
    "keypress",
    "scroll",
    "select",
    "screenshot",
    "highlight",
    "eval",
];

impl AppServer {
    /// `chrome/status` — current browser-bridge connection snapshot.
    pub(crate) async fn handle_chrome_status(&self) -> Result<serde_json::Value, JsonRpcError> {
        Ok(status_value())
    }

    /// `chrome/enable` — enable Chrome tools for the session, optionally setting mode.
    pub(crate) async fn handle_chrome_enable(
        &self,
        params: ChromeEnableParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let bridge = bridge();
        if let Some(mode) = params.mode.as_deref() {
            let mode = parse_mode(mode)?;
            bridge.set_mode(mode);
        }
        bridge.set_enabled(true);
        Ok(status_value())
    }

    /// `chrome/disable` — disable Chrome tools for the session.
    pub(crate) async fn handle_chrome_disable(&self) -> Result<serde_json::Value, JsonRpcError> {
        bridge().set_enabled(false);
        Ok(status_value())
    }

    /// `chrome/setMode` — set the permission mode (observe/assist/control).
    pub(crate) async fn handle_chrome_set_mode(
        &self,
        params: ChromeSetModeParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let mode = parse_mode(&params.mode)?;
        bridge().set_mode(mode);
        Ok(status_value())
    }

    /// `chrome/reconnect` — report current status (connection is transport-driven).
    pub(crate) async fn handle_chrome_reconnect(&self) -> Result<serde_json::Value, JsonRpcError> {
        Ok(status_value())
    }

    /// `chrome/browsers/list` — list browsers, or an empty list when disconnected.
    pub(crate) async fn handle_chrome_browsers_list(
        &self,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let bridge = bridge();
        if !bridge.status().connected {
            return Ok(serde_json::json!({ "browsers": [] }));
        }
        dispatch(ChromeCommand::new("browsers/list")).await
    }

    /// `chrome/tabs/list` — list tabs visible to the connected extension.
    pub(crate) async fn handle_chrome_tabs_list(&self) -> Result<serde_json::Value, JsonRpcError> {
        dispatch(ChromeCommand::new("tabs/list")).await
    }

    /// `chrome/tabs/activate` — focus a tab by id.
    pub(crate) async fn handle_chrome_tabs_activate(
        &self,
        params: ChromeTabActivateParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let body = serde_json::to_value(&params).map_err(invalid_params)?;
        dispatch(ChromeCommand::with_params("tab/activate", body)).await
    }

    /// `chrome/tabs/navigate` — navigate a tab (active tab when `tabId` omitted).
    pub(crate) async fn handle_chrome_tabs_navigate(
        &self,
        params: ChromeNavigateParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let body = serde_json::to_value(&params).map_err(invalid_params)?;
        dispatch(ChromeCommand::with_params("tab/navigate", body)).await
    }

    /// `chrome/page/snapshot` — capture a page snapshot.
    pub(crate) async fn handle_chrome_page_snapshot(
        &self,
        params: ChromePageSnapshotParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let body = serde_json::to_value(&params).map_err(invalid_params)?;
        dispatch(ChromeCommand::with_params("page/snapshot", body)).await
    }

    /// `chrome/page/action` — run a page action, mapped to wire kind `page/<action>`.
    pub(crate) async fn handle_chrome_page_action(
        &self,
        params: ChromePageActionParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let action = params.action.trim();
        if !PAGE_ACTIONS.contains(&action) {
            return Err(invalid_params(format!(
                "unsupported page action {action:?}; expected one of {}",
                PAGE_ACTIONS.join(", ")
            )));
        }
        let kind = format!("page/{action}");
        let body = serde_json::Value::Object(params.extra);
        dispatch(ChromeCommand::with_params(kind, body)).await
    }

    /// `chrome/debug/console` — read recent console messages.
    pub(crate) async fn handle_chrome_debug_console(
        &self,
        params: ChromeDebugReadParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let body = serde_json::to_value(&params).map_err(invalid_params)?;
        dispatch(ChromeCommand::with_params("debug/console/read", body)).await
    }

    /// `chrome/debug/network` — read recent network metadata.
    pub(crate) async fn handle_chrome_debug_network(
        &self,
        params: ChromeDebugReadParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let body = serde_json::to_value(&params).map_err(invalid_params)?;
        dispatch(ChromeCommand::with_params("debug/network/read", body)).await
    }

    /// `chrome/permissions/list` — read stored site permissions.
    pub(crate) async fn handle_chrome_permissions_list(
        &self,
        params: ChromePermissionsListParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let body = serde_json::to_value(&params).map_err(invalid_params)?;
        dispatch(ChromeCommand::with_params("permissions/get", body)).await
    }

    /// `chrome/permissions/update` — update stored site permissions for an origin.
    pub(crate) async fn handle_chrome_permissions_update(
        &self,
        params: ChromePermissionsUpdateParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let body = serde_json::to_value(&params).map_err(invalid_params)?;
        dispatch(ChromeCommand::with_params("permissions/set", body)).await
    }
}

/// Serialize the current bridge status as a JSON-RPC result value.
fn status_value() -> serde_json::Value {
    serde_json::to_value(bridge().status()).expect("chrome status serializes")
}

/// Forward a command to the connected extension and surface failures as JSON-RPC errors.
async fn dispatch(command: ChromeCommand) -> Result<serde_json::Value, JsonRpcError> {
    bridge().dispatch(command).await.map_err(chrome_error)
}

fn parse_mode(value: &str) -> Result<ChromePermissionMode, JsonRpcError> {
    ChromePermissionMode::parse(value).ok_or_else(|| {
        invalid_params(format!(
            "unknown chrome permission mode {value:?}; expected observe, assist, or control"
        ))
    })
}

/// Map a [`ChromeError`] to a JSON-RPC error in the chrome-reserved -3201x band.
fn chrome_error(err: ChromeError) -> JsonRpcError {
    let code = match err {
        ChromeError::NotConnected => -32010,
        ChromeError::Disabled => -32011,
        ChromeError::Rejected(_) => -32012,
        ChromeError::Timeout => -32013,
        ChromeError::Disconnected => -32014,
        ChromeError::Remote(_) => -32015,
    };
    JsonRpcError {
        code,
        message: err.to_string(),
        data: None,
    }
}

fn invalid_params(err: impl std::fmt::Display) -> JsonRpcError {
    JsonRpcError {
        code: -32602,
        message: format!("Invalid params: {err}"),
        data: None,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use roder_core::{Runtime, RuntimeConfig};
    use roder_extension_host::{DefaultRegistryConfig, build_default_registry};
    use roder_protocol::{JsonRpcRequest, JsonRpcResponse};

    use crate::server::AppServer;

    fn test_server() -> Arc<AppServer> {
        let registry = build_default_registry(DefaultRegistryConfig::default()).unwrap();
        let runtime = Arc::new(Runtime::new(registry, RuntimeConfig::default()).unwrap());
        Arc::new(AppServer::new(runtime))
    }

    fn request(method: &str, params: Option<serde_json::Value>) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(method)),
            method: method.to_string(),
            params,
        }
    }

    fn result(response: JsonRpcResponse) -> serde_json::Value {
        assert!(response.error.is_none(), "unexpected error: {response:?}");
        response.result.expect("result present")
    }

    #[tokio::test]
    async fn chrome_status_reports_disconnected_shape() {
        let server = test_server();
        let response = server.handle_request(request("chrome/status", None)).await;
        let value = result(response);
        assert_eq!(value["connected"], serde_json::json!(false));
        assert_eq!(value["clientCount"], serde_json::json!(0));
        assert!(value.get("mode").is_some());
        assert!(value["capabilities"].is_array());
    }

    #[tokio::test]
    async fn chrome_enable_sets_enabled_and_mode_in_status() {
        let server = test_server();
        let response = server
            .handle_request(request(
                "chrome/enable",
                Some(serde_json::json!({ "mode": "control" })),
            ))
            .await;
        let value = result(response);
        assert_eq!(value["enabled"], serde_json::json!(true));
        assert_eq!(value["mode"], serde_json::json!("control"));

        // Status method should reflect the same enabled state.
        let status = result(server.handle_request(request("chrome/status", None)).await);
        assert_eq!(status["enabled"], serde_json::json!(true));

        // Leave the global bridge disabled for any sibling tests in this process.
        let _ = server
            .handle_request(request("chrome/disable", None))
            .await;
    }

    #[tokio::test]
    async fn chrome_enable_rejects_unknown_mode() {
        let server = test_server();
        let response = server
            .handle_request(request(
                "chrome/enable",
                Some(serde_json::json!({ "mode": "bogus" })),
            ))
            .await;
        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, -32602);
    }
}
