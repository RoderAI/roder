---
roder-app-server: minor
roder-tui: minor
roder: patch
---

# Persist browser pairing across Roder restarts

Pairing the browser extension used to last exactly as long as one Roder process:
`/remote` minted a fresh random token on every start and listened on an
OS-assigned port, so the endpoint and token the extension had stored were both
dead after a restart and the user had to walk the `/pair` flow again.

The token and the bound loopback port are now stored in
`<config-dir>/remote-pairing.json` (owner-only, never logged) and reused, and a
Roder that finds that file brings the listener back up on the same endpoint at
startup. Pair once and later runs reconnect with no user action. `/remote
regenerate` — new alias `/remote unpair` — rotates the token and revokes every
paired browser; a remembered port that is already taken falls back to an
ephemeral port, which is then remembered in turn.

Also fixes a stack overflow that aborted the TUI on its first turn. `main` used
a bare `#[tokio::main]`, so the agent-loop future ran on a worker thread with
tokio's 2 MiB default stack — the Windows main, the roadmap TUI and the
app-server already spawn themselves on 32 MiB for exactly this reason. All the
Roder runtimes now set that stack size for their worker threads too.
