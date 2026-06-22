# roder-app-server-node

Agent-node control server and `RemoteAppClient` controller client for
[Roder](https://roder.sh), split out of `roder-app-server`.

Contains the encrypted agent-node server (`agent_node`) plus the
`RemoteAppClient`/`RemoteNodeConnection` controller transport. Only the CLI
binary and node integration tests depend on it, so keeping it in a separate
crate moves the TLS/mTLS-heavy code off the `roder-app-server` compile unit.
