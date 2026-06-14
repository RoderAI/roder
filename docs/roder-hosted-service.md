# Roder Hosted Multi-Tenant Service

`roder-hosted` (crate `roder-dist-hosted`) runs Roder as a hosted
multi-tenant gateway: many tenants share one process while each tenant gets
its **own runtime** — own thread/artifact stores, own automation database,
own notification stream — so cross-tenant access is impossible by
construction. The TUI is not part of this distribution; clients speak the
gateway WebSocket protocol.

Do not confuse this with the other two remote modes:

| Mode | Trust model | Use |
| --- | --- | --- |
| Trusted LAN remote (`app-server --listen`) | One user, one bearer token, trusted network | Personal LAN access |
| Agent node (`roder agent-node serve`) | Machine-to-machine mTLS with pinned fingerprints | Driving one remote Roder from another |
| **Hosted service (`roder-hosted`)** | Multi-tenant credentials, per-request authorization, per-tenant runtimes | Serving many users/tenants |

## Configuration

`roder-hosted --config /etc/roder/hosted.toml`:

```toml
listen = "127.0.0.1:7900"        # keep private; TLS terminates at your proxy
data_root = "/var/lib/roder-hosted"
allow_local_workspaces = false    # hosted default: execution needs a runner destination
idle_ttl_secs = 900

[[tenants]]
id = "acme"
display_name = "Acme"

[[static_keys]]
token_env = "RODER_HOSTED_KEY_ACME_ADMIN"  # env reference; raw keys rejected
tenant = "acme"
user = "ops"
role = "tenant_admin"                       # member | tenant_admin | system_admin
scopes = ["read", "write", "admin"]

[rate_limit]
burst = 60
per_second = 10.0
max_request_bytes = 1048576
```

Env overrides: `RODER_HOSTED_LISTEN`, `RODER_HOSTED_DATA_ROOT`,
`RODER_HOSTED_ALLOW_LOCAL_WORKSPACES`. Config validation rejects inline
secrets, unknown tenants/roles/scopes; startup fails closed when a
referenced key env var is unset. Logs print only redacted summaries.

## Security model (current state)

- **Auth before dispatch**: every connection authenticates at the WebSocket
  handshake (`Authorization: Bearer …`; query-string credentials always
  rejected). Static `rk_test_*` keys and hashed `rk_sa_*` service-account
  keys are supported; JWT/JWKS is deferred pending a dependency/IdP
  decision.
- **Authorization per request**: read-only methods need `read`, mutating
  methods `write`, `hosted/*` administration `admin` + tenant-admin role;
  cross-tenant listing is system-admin only; unknown methods are denied.
- **Tenant isolation by construction**: per-tenant runtimes and stores
  under `data_root/<tenant>/`; notifications come from the tenant's own
  app-server.
- **Limits and audit**: per-tenant/principal token-bucket rate limits,
  frame-size limits, and an append-only redacted audit JSONL
  (`hosted/audit/list` exposes a tenant's own records).
- **Hosted workspace policy**: with `allow_local_workspaces = false`
  (default), `workspace/create` and `thread/start` with a `cwd` are denied;
  configure runner destinations for execution.
- **Hooks**: tenant-scoped webhooks with HMAC-SHA256 signatures
  (`x-roder-signature`), bounded retries, circuit breaker, and dead-letter
  records; signing secrets are env references resolved at delivery time.

Production warnings: terminate TLS in front of the gateway and never expose
the plain listener; store key env values in a secret manager; back up
`data_root` (per-tenant stores + audit log) atomically per tenant
directory; deleting a tenant directory is the current tenant-offboarding
mechanism. Migrations are per-store (JSONL files are append-only; the
analytics/automation SQLite files migrate on open).

## SDK access

Both SDKs connect to hosted Roder with the normal app-server protocol plus
hosted helpers — no separate protocol:

```ts
import { HostedClient } from "@roderai/sdk";
const hosted = await HostedClient.connect({ url, token }); // or tokenProvider / headers
await hosted.whoami();
const key = await hosted.createServiceAccount("ci"); // token shown once
await hosted.reconnect(); // fresh token from the provider; nothing is replayed
```

```python
from roder_sdk import HostedClient
hosted = await HostedClient.connect(url, token=token)  # or token_provider= / headers=
await hosted.whoami()
```

Raw JSON-RPC stays available via `hosted.client` for forward-compatible
hosted methods. Examples: `sdk/examples/typescript/hosted-service.ts`,
`sdk/examples/python/hosted_service.py`; live smokes are gated behind
`RODER_HOSTED_SDK_LIVE=1` (`sdk/scripts/hosted-live-smoke.ts`,
`sdk/scripts/hosted_live_smoke.py`).

## Operations runbook

- **Key rotation**: add the new key env var to a new `[[static_keys]]`
  entry and restart (startup fails closed if any referenced var is unset);
  distribute the new key; remove the old entry and restart again. Service
  accounts rotate by minting (`hosted/service_accounts/create`) then
  revoking the old key id; revocation takes effect for new connections
  immediately.
- **Hook delivery replay**: failed deliveries retry automatically with
  bounded backoff; terminal failures land in the dead-letter list with
  coarse error classes (`timeout`, `http_5xx`, …). The current replay
  mechanism is re-sending the originating action after fixing the target;
  dead-letter records identify the hook and event kind. The per-hook
  circuit breaker re-probes after its cooldown without operator action.
- **Audit export**: the audit log is append-only JSONL at the configured
  `audit_log` path (default `<data_root>/audit.jsonl`); ship it with any
  log forwarder. Tenant admins can pull their own slice via
  `hosted/audit/list`. Records carry credential ids and coarse reasons,
  never secrets.
- **Backups**: snapshot `data_root` per tenant directory (thread stores,
  automation DBs) plus the audit JSONL; per-tenant directories are
  self-contained, so restores can be per-tenant.
- **Migrations**: JSONL stores are append-only; SQLite stores migrate on
  open; the PostgreSQL session store migrates at connect. Upgrade by
  stopping the service (idle eviction + bounded graceful shutdown protect
  active turns), replacing the binary, and restarting.
- **Incident response**: revoke the affected credential (static key env
  removal + restart, or `hosted/service_accounts/revoke`), inspect
  `audit.jsonl` for `auth_failed`/`method_denied`/`rate_limited` patterns
  by credential id, and rotate hook signing secrets by updating the
  referenced env values. The gateway never logs token material, so leaked
  logs do not leak credentials.

## Deployment examples

`deploy/roder-hosted/` contains secret-free examples for Docker Compose,
systemd, Nomad, and Kubernetes. All of them inject key material via env
files/secret stores and assume a TLS-terminating proxy.

## Verification

```sh
cargo test -p roder-config hosted
cargo test -p roder-dist-hosted              # launches the gateway with fake auth
cargo test -p roder-app-server --test hosted_auth --test hosted_gateway --test hosted_hooks
```

The acceptance test boots the service from a `HostedConfig`, rejects
unauthenticated and bad-token WebSocket connects, answers authenticated
`initialize` + `hosted/whoami`, and verifies the audit JSONL contains
auth events but no raw tokens.
