# roder-app-server-hosted

Hosted multi-tenant gateway for the Roder app-server.

Wraps a `roder_app_server::AppServer` with tenant/principal authentication,
method authorization, per-tenant rate/size limits, gateway-level thread
ownership isolation, audit records, and webhook delivery — all enforced before
JSON-RPC dispatch. Local single-user mode never constructs any of this.

Extracted from `roder-app-server` so the multi-tenant gateway (only needed by
the hosted distribution, `roder-dist-hosted`) no longer compiles as part of the
local app-server crate that sits on the `roder` binary's critical build path.
