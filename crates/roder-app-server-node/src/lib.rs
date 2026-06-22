//! Agent-node client surface for Roder (roadmap phase 67).
//!
//! Hosts the encrypted agent-node control server (`agent_node`) and the
//! `RemoteAppClient`/`RemoteNodeConnection` controller client that connects to
//! a node over mandatory TLS. Split out of `roder-app-server` so the
//! TLS/mTLS-heavy node + controller code (only the CLI binary and node tests
//! consume it) compiles in parallel with the rest of the workspace instead of
//! inside the app-server translation unit. The local-server runtime and the
//! server-side WebSocket transport (`roder_app_server::remote`) stay in
//! `roder-app-server`.

pub mod agent_node;
pub mod remote_client;

pub use remote_client::{RemoteAppClient, RemoteNodeConnection};
