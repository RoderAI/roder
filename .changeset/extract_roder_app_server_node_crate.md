---
roder-app-server-node: minor
roder-app-server: minor
roder-cli: patch
---

# Extract agent-node client into roder-app-server-node

The encrypted agent-node control server (`agent_node`) and the
`RemoteAppClient`/`RemoteNodeConnection` controller client (`remote_client`)
moved out of `roder-app-server` into a new `roder-app-server-node` crate that
depends on `roder-app-server`.

These ~1.2k lines (TLS/mTLS-heavy node + controller code) are consumed only by
the `roder` CLI binary and the node integration tests — crucially **not** by
`roder-tui`. Keeping them inside `roder-app-server` forced them to compile
serially within that crate's translation unit, which sits on the binary's
critical build path ahead of `roder-tui`. Splitting them out lets them compile
in parallel with `roder-tui` and trims now-unused TLS dependencies
(`rustls`, `tokio-rustls`, `rcgen`, `rustls-pemfile`, `reqwest`) from
`roder-app-server`. The server-side WebSocket transport
(`roder_app_server::remote`), which `roder-tui` does depend on, stays in
`roder-app-server`.

**Breaking:** `roder_app_server::agent_node::*` is now
`roder_app_server_node::agent_node::*`, and the `roder_app_server::{RemoteAppClient,
RemoteNodeConnection}` re-exports are now `roder_app_server_node::{RemoteAppClient,
RemoteNodeConnection}`.
