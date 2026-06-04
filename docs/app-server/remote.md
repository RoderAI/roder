# Roder Remote App-Server

Remote app-server mode exposes the same JSON-RPC control plane over an authenticated WebSocket for same-network or Tailscale clients.

Start it explicitly:

```sh
roder app-server --remote
roder app-server --remote --listen ws://0.0.0.0:0
roder app-server --remote --listen ws://100.x.y.z:0
roder app-server --remote --auth-token env:RODER_REMOTE_TOKEN
roder app-server --remote --print-qr=false
```

Without `--remote`, `roder app-server` keeps using `stdio://`. Remote mode defaults to `ws://0.0.0.0:0` so the operating system picks a free port and Roder prints a terminal QR code, usable connection URLs, and a `roder://connect` pairing link.

## Authentication

Remote WebSocket upgrades require a bearer token during the handshake. Native clients should send:

```text
Authorization: Bearer <token>
```

Browser-constrained clients that cannot set custom WebSocket headers can use subprotocol auth:

```text
Sec-WebSocket-Protocol: roder.remote.v1, bearer.<token>
```

Tokens are not accepted in WebSocket query parameters. The pairing payload includes the token inside the encoded `roder://connect?payload=...` value, but logs, app-server events, and TUI summaries use the token preview only.

## Browser extension bridge

The Roder browser extension pairs over this same remote WebSocket listener and authenticates with the subprotocol bearer flow (`Sec-WebSocket-Protocol: roder.remote.v1, bearer.<token>`), since extensions cannot set custom WebSocket headers. Once connected, the extension sends a `hello` browser-bridge frame (a `{ "type": ... }` envelope, not JSON-RPC). The transport registers it with the process-global Chrome bridge (`roder_api::chrome`) and forwards its command stream. The `chrome/*` app-server methods (see `api.md`) then bridge JSON-RPC requests to the connected extension by pushing `{ type, id, ...params }` command frames and awaiting the matching `{ type: "command/result", id, ok, result, error }` reply. Browser page content, console output, and network metadata returned through the bridge are untrusted and are passed through verbatim; only the bearer-token preview (never the full token) and connection metadata are logged.

## Security Model

Remote mode is intended for a trusted LAN or Tailscale network. Raw `ws://` over a LAN IP is not TLS-protected; use Tailscale or another trusted private tunnel when possible. Do not expose the remote app-server directly to the public internet.

`GET /readyz` and `GET /healthz` return unauthenticated `200 OK` health responses on the same listener. WebSocket upgrades still require bearer auth in remote mode.

The local event stream records remote server starts and stops, auth failures, client connects, and client disconnects as sanitized events:

- `remote/serverStarted`
- `remote/serverStopped`
- `remote/authFailed`
- `remote/clientConnected`
- `remote/clientDisconnected`

These events include connection metadata and token previews only, never the full bearer token.

## TUI

Use `/remote` or `Ctrl+P -> Remote control` to open the remote pairing workflow. The TUI surface shows connection URLs, token preview, pairing payload, connected-client count, and a warning when the selected URL is a LAN WebSocket without TLS.
