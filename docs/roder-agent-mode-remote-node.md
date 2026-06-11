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

## Connection profiles and the controller TUI

Configure nodes once in `config.toml` and connect with the full TUI:

```toml
[[agent_nodes]]
name = "studio"
address = "studio.local:7878"
fingerprint = "<node cert sha256>"   # printed by `roder agent-node serve`
token_env = "RODER_STUDIO_TOKEN"     # only consulted for first enrollment
```

```sh
roder agent-node profiles            # list configured nodes
roder agent-node connect studio      # open the TUI against the node
```

Pairing tokens are never stored in config — `token_env` names an
environment variable holding a one-time token; after enrollment the
controller certificate alone authorizes. The TUI renders a remote-node
banner (node name/id, auth mode, certificate fingerprint, node workspace)
so it stays unmistakable that turns, tools, and files run on the node, not
the controller terminal. The model defaults to the node's default model
(`--model` overrides).

Any client can also ask `node/status` over JSON-RPC: locally-served
app-servers answer `{ "served": false }`, agent nodes return the node
identity with the per-connection `authMode`. Enrollment and revocation are
deliberately CLI-only operations on the node host, not public app-server
methods.

## Trust management and recovery

- `roder agent-node trust list` (on the node) shows enrolled controller
  fingerprints.
- `roder agent-node trust revoke <fingerprint>` revokes a controller;
  revoked fingerprints cannot re-enroll and a fresh controller identity +
  pairing token is required.
- Lost node certificate: delete the node identity under
  `<config>/agent-node/` and restart `serve`; a new fingerprint is
  generated, so update controller profiles.
- Lost controller certificate: delete `<config>/agent-node/controller/`;
  the next `connect`/`connect-check` generates a new identity which must be
  re-enrolled with a fresh pairing token.

## Deployment notes

- Run the node under a process supervisor (launchd/systemd) with
  `roder agent-node serve --listen <addr>:<port> --name <label>`; mint
  pairing tokens only when enrolling a new controller — they are
  single-use and short-lived, not long-lived secrets.
- There is no plaintext mode. For internet-facing nodes prefer a
  WireGuard/Tailscale/SSH tunnel and keep the listener on a private
  address; fingerprint pinning protects against MITM either way.
- The trust store and certificates live under `<config>/agent-node/`; back
  this directory up to preserve enrollment across reinstalls and protect it
  like an SSH host key directory.

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

Implemented (phase 67 Stages 1–5): TLS/mTLS transport with fingerprint
pinning both ways, pairing-token enrollment and revocation,
`RemoteAppClient`, the `node/status` protocol method, `[[agent_nodes]]`
connection profiles, the `serve` / `connect` / `connect-check` /
`profiles` / `trust` CLI, the controller TUI with a remote-node authority
banner, and the trust-recovery/deployment guidance above.

Open: automated certificate-rotation tooling (today rotation = replace the
identity directory and re-enroll) and live multi-machine deployment
recipes beyond loopback (the protocol carries no machine-local
assumptions; loopback smoke + offline tests are the current evidence).
