---
roder-app-server-core: minor
roder-app-server: minor
roder-app-server-node: patch
roder-tui: patch
roder-cli: patch
---

# Extract client/transcript layer into roder-app-server-core

The `AppClient` trait, its `AppEventReceiver`/`AppNotificationReceiver`
receivers, and the `TranscriptRecorder`/`RecordingAppClient` transcript layer
moved out of `roder-app-server` into a new lightweight `roder-app-server-core`
crate. None of these depend on the heavy `AppServer` implementation in
`server.rs`, so consumers (`roder-tui`, `roder-cli`, `roder-app-server-node`)
can type-check against the trait surface in parallel with the server crate.

This is the first step toward decoupling `roder-tui`'s library build from
`server.rs`. **Breaking:** `roder_app_server::{AppClient, AppEventReceiver,
AppNotificationReceiver, transcript::*}` are now in `roder_app_server_core`;
`LocalAppClient` and `AppServer` remain in `roder-app-server`.
