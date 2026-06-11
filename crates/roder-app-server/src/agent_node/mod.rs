//! Agent-node mode (roadmap phase 67): one Roder instance runs the
//! authoritative runtime/app-server and another connects as a controller
//! over mandatory TLS with mTLS-pinned identity. See
//! `docs/roder-agent-mode-remote-node.md`.

pub mod auth;
pub mod server;
pub mod tls;

pub use auth::{ControllerTrust, DEFAULT_PAIRING_TTL, PairingTokens};
pub use server::{
    AGENT_NODE_PROTOCOL, AgentNodeController, AgentNodeHandle, AgentNodeOptions,
    NODE_EVENT_METHOD, load_or_generate_identity, serve_agent_node,
};
pub use tls::{TlsIdentity, client_tls_config, fingerprint_from_pem, generate_identity};
