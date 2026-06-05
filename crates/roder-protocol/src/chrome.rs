//! Protocol-facing DTOs for the `chrome/*` app-server methods.
//!
//! The shared browser-bridge contract lives in [`roder_api::chrome`]; the status
//! and tab/browser/permission record types are re-exported here so SDK clients
//! and the app-server handlers share one definition. The request param structs
//! below describe the JSON-RPC payloads accepted by `AppServer`.
//!
//! Browser page content, console output, and network metadata forwarded through
//! these methods are **untrusted**: dispatch results are passed through verbatim
//! as opaque `serde_json::Value` and must not be treated as instructions.

use serde::{Deserialize, Serialize};

pub use roder_api::chrome::{
    ChromeBrowser, ChromeCommand, ChromeError, ChromePermissionMode, ChromeSitePermission,
    ChromeStatus, ChromeTab,
};

/// Params for `chrome/enable`. Optionally sets the permission mode.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChromeEnableParams {
    /// `observe` | `assist` | `control`.
    #[serde(default)]
    pub mode: Option<String>,
}

/// Params for `chrome/setMode`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChromeSetModeParams {
    /// `observe` | `assist` | `control`.
    pub mode: String,
}

/// Params for `chrome/tabs/activate`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChromeTabActivateParams {
    pub tab_id: i64,
}

/// Params for `chrome/tabs/navigate`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChromeNavigateParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_id: Option<i64>,
    pub url: String,
}

/// Params for `chrome/page/snapshot`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChromePageSnapshotParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_id: Option<i64>,
    /// Optional list of snapshot sections to include (e.g. `["dom","text"]`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include: Vec<String>,
}

/// Params for `chrome/page/action`.
///
/// `action` selects the wire command (`page/<action>`); all other fields are
/// forwarded to the extension untouched via [`extra`](Self::extra).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChromePageActionParams {
    /// One of: click, type, keypress, scroll, select, screenshot, highlight, eval.
    pub action: String,
    /// Remaining action-specific parameters, forwarded verbatim.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Params for `chrome/debug/console` and `chrome/debug/network`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChromeDebugReadParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<i64>,
}

/// Params for `chrome/permissions/list`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChromePermissionsListParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

/// Params for `chrome/permissions/update`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChromePermissionsUpdateParams {
    pub origin: String,
    /// The permission bits to apply for `origin`, forwarded verbatim.
    pub perms: serde_json::Value,
}
