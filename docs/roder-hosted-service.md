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
