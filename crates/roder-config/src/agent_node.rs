//! `[[agent_nodes]]` connection profiles (roadmap phase 67, Stage 4).
//!
//! Profiles carry the address and the pinned node certificate fingerprint.
//! Pairing tokens are never stored in config — `token_env` names an
//! environment variable that holds a one-time token for first enrollment;
//! after enrollment the controller certificate alone authorizes.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentNodeProfile {
    /// Profile name used by `roder agent-node connect <name>`.
    pub name: String,
    /// `host:port` of the node's TLS listener.
    pub address: String,
    /// Pinned SHA-256 fingerprint of the node certificate (hex).
    pub fingerprint: String,
    /// Env var holding a one-time pairing token for bootstrap enrollment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_env: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_node_profiles_parse_from_toml() {
        #[derive(Deserialize)]
        struct Doc {
            #[serde(default)]
            agent_nodes: Vec<AgentNodeProfile>,
        }
        let doc: Doc = toml::from_str(
            r#"
            [[agent_nodes]]
            name = "studio"
            address = "studio.local:7470"
            fingerprint = "abc123"
            token_env = "RODER_STUDIO_TOKEN"

            [[agent_nodes]]
            name = "colo"
            address = "10.0.0.5:7470"
            fingerprint = "def456"
            "#,
        )
        .unwrap();
        assert_eq!(doc.agent_nodes.len(), 2);
        assert_eq!(doc.agent_nodes[0].token_env.as_deref(), Some("RODER_STUDIO_TOKEN"));
        assert!(doc.agent_nodes[1].token_env.is_none());
    }
}
