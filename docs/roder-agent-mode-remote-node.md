# Agent Mode: Remote Node

Agent-node mode runs one Roder instance as the authoritative agent server
(runtime, app-server, tools, sessions, processes, extensions) and lets
another Roder instance control it over a secure connection through the
same `AppClient` abstraction the local TUI uses.

This is different from remote runners: a remote runner executes workspace
commands beneath a *local* runtime, while an agent node owns the whole
runtime remotely.

## Security model

- Control transport is always `wss://` (rustls). There is no unencrypted
  agent-node mode, including on LAN.
- mTLS is canonical: the node's certificate identifies the node (pinned by
  fingerprint on the controller side) and the controller's certificate
  identifies the controller (fingerprint enrolled on the node side).
- Pairing tokens exist only to bootstrap enrollment: single-use,
  short-lived (10 minutes), stored hashed, printed once at node startup,
  and shown elsewhere only by preview. A connection presenting an
  unenrolled certificate plus a valid token enrolls that certificate and
  is immediately trusted; reconnects use mTLS only.
- Tokens are never accepted in query strings.
- Trust state (node identity, enrolled controller fingerprints, revoked
  fingerprints) lives under `~/.roder/agent-node/`. Private keys are
  written `0600` and never leave the machine that generated them.
- Revoked controller fingerprints are refused and cannot re-enroll.

## Running a node

```sh
roder agent-node serve --listen 0.0.0.0:7878 --name my-vps
```

Startup prints the bound `wss://` URL, node id, certificate fingerprint,
and a single-use pairing token with copy-paste enrollment instructions.
Identity persists across restarts under `~/.roder/agent-node/`.

## Connecting a controller

```sh
roder agent-node connect-check \
  --address my-vps:7878 \
  --fingerprint <node-cert-sha256> \
  --token <pairing-token>     # first time only
```

The first connection enrolls the controller certificate (generated and
persisted under `~/.roder/agent-node/controller/`); subsequent connections
authenticate with mTLS alone. `connect-check` calls `initialize` and
`thread/list` and prints the node identity (`nodeId`, `name`, `authMode`,
`protocolVersion: roder.agent-node.v1`).

Programmatic controllers use `RemoteAppClient` (implements `AppClient`):
JSON-RPC requests are id-correlated with bounded in-flight slots and
timeouts; protocol notifications and runtime event envelopes (`node/event`
frames) stream over the same authenticated connection; on disconnect,
pending requests fail with explicit errors (mutating requests are never
silently replayed) and the client reconnects with capped backoff using
mTLS only.

## Verification (offline)

```sh
cargo test -p roder-app-server --test agent_node_security
cargo test -p roder-app-server --test remote_app_client
cargo test -p roder-cli --bin roder agent_node
```

Security tests prove: enrolled mTLS connects; missing/unenrolled/revoked
client certificates fail before request dispatch; wrong node fingerprints
fail TLS trust on the controller side; pairing tokens are single-use and
expire; wrong tokens fail; query-string tokens are rejected without being
read. The controller tests drive a full thread/turn flow with streamed
events over loopback mTLS and prove explicit-failure + reconnect behavior.

## Current status and remaining work

Implemented (phase 67 Stages 1–2): TLS/mTLS transport, fingerprint
pinning both ways, pairing-token enrollment, revocation, the agent-node
serve/connect-check CLI, and `RemoteAppClient`.

Open (Stages 3–5): `node/*` public protocol methods and schema entries,
named connection profiles in config, TUI remote-authority status
rendering, certificate rotation commands, and deployment recipes
(Tailscale/SSH tunnel/public DNS).
