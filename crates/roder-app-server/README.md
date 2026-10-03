# roder-app-server

`roder-app-server` is the local app-server runtime for [Roder](https://roder.sh).

## What It Does

It exposes the Roder runtime over JSON-RPC and WebSocket transports for the TUI, SDKs, ACP adapter, remote nodes, and integration tests.

## How It Fits Into Roder

Roder is an agentic software development system with a Rust CLI/TUI, a JSON-RPC app-server, SDKs, package resources, and first-party runtime extensions. This package is released as part of that workspace so downstream users can depend on the same component boundaries that Roder itself uses.

## Links

- Roder website: https://roder.sh
- Repository: https://github.com/RoderAI/roder

## Publishing

This package is versioned and published with the Roder workspace. Before publishing, run:

```sh
make registry-readmes
python3 scripts/generate-knope-config.py --check
```

### Routing hosted sessions to their owner

A `TenantAppServerFactory` can return `HostedRuntimeRedirect` when its durable
ownership registry reports a different live owner. The gateway forwards the
connection to that endpoint without constructing a local runtime. The owner
revalidates the original bearer and the authenticated tenant, then applies its
normal request authorization, limits, and deployment policy. Forwarding is
limited to one hop; failed or broken connections are not retried or replayed.
Rotated credentials for the same tenant therefore resolve through the same
ownership record rather than a hash of the credential.

The host must validate that registry endpoints belong to its trusted replica
network. Replicas must share authentication and policy, and their private
transport must protect credentials. Client-supplied endpoints are never valid
owner routes. The runtime pool discards permanently revoked cached runtimes on
reconnect so the factory can resolve current ownership again.

This routing primitive does not acquire leases, transfer active turns, or
reconcile external side effects. Hosts must bind an owner-fenced store and
`RuntimeExecutionLease`, install renewal supervision, and coordinate graceful
drain before removing an owning replica.

For a planned handoff, first stop new inbound work, then poll
`AppServer::release_idle_runtime_owner`. It returns false without interrupting
busy work. Success seals local admission and confirms release of the exact
durable generation. Lost authority, failed lifecycle persistence, timeout, or
unconfirmed release are errors, not successful handoff receipts. This differs
from shutdown drain, which requests interruption. The host still coordinates
traffic removal, replacement ownership, and final process termination.

At replica level, `HostedRuntimePool::begin_owner_drain` closes new-work admission
while preserving resident-owner reconnects, tool results, approvals, and recovery
messages. `/readyz` returns 503 during drain; `/healthz` stays healthy.
`poll_owner_drain` reports remaining tenants and forwarded sockets. Both counts
must be zero, with no error, before termination. Unknown releases remain visible
and cannot be removed by idle eviction. `resume_owner_admission` restores admission
for rollback, reconstructing released runtimes through the ownership factory.
Forwarded sockets are counted until they close; moving those connections without
interrupting remote work remains a host coordination responsibility.

Embedders may install `HostedGatewayOptions::lifecycle` to handle signed
`POST /lifecycle` requests on the same port. The handler receives the exact body
and `X-Roder-Lifecycle-Signature`; it must authenticate and validate commands
before changing pool state. The transport bounds headers/body and read time,
rejects duplicate length/signature headers and transfer encoding, and returns
404 when no handler is installed. WebSocket authentication remains separate.
