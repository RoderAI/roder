//! Agent-node protocol DTOs (roadmap phase 67, Stage 3).
//!
//! `node/status` lets any client ask which node is serving it and how the
//! connection is authorized. Locally-served app-servers answer with
//! `served: false`; agent-node servers fill the identity that is also
//! injected into `initialize` metadata. Enrollment and revocation are
//! deliberately CLI-only operations on the node host (they mutate the
//! node's local trust store), not public app-server methods.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NodeIdentity {
    pub node_id: String,
    pub name: String,
    /// SHA-256 fingerprint of the node certificate (hex).
    pub fingerprint: String,
    /// `mtls` or `pairing-token-enrolled` for the current connection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_mode: Option<String>,
    pub protocol_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NodeStatusResult {
    /// True when this app-server is being served as a remote agent node.
    pub served: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<NodeIdentity>,
}
