---
roder-app-server-hosted: minor
roder-app-server: minor
roder-dist-hosted: patch
---

# Extract hosted gateway into roder-app-server-hosted

The hosted multi-tenant gateway (`auth`, `authorization`, `tenant`,
`rate_limit`, `runtime_pool`, `hooks`, `hook_delivery`, `audit`, and the
WebSocket `gateway`) moved out of `roder-app-server` into a new
`roder-app-server-hosted` crate that depends on `roder-app-server`.

The hosted gateway is only used by the `roder-dist-hosted` distribution and is
never reachable from the local `roder` binary, so removing it (~1.7k lines plus
its four integration-test binaries) shrinks the `roder-app-server` crate that
sits on the binary's critical build path and lets the gateway compile in
parallel. **Breaking:** `roder_app_server::hosted::*` is now
`roder_app_server_hosted::*`; `roder-dist-hosted` is updated accordingly.
