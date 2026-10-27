use serde::{Deserialize, Serialize};

/// Provider-owned ceilings for OpenAI-compatible Responses endpoints.
/// Omitted settings retain the endpoint's built-in defaults.
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResponsesProviderCapabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_compaction: Option<RemoteCompaction>,
    /// Whether hosted search may fetch live external content. `false` limits
    /// search to cached results; it cannot enable search disabled by the caller.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_web_access: Option<bool>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RemoteCompaction {
    Unsupported,
    V2,
}
