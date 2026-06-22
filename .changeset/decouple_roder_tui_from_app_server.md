---
roder-tui: minor
roder-cli: patch
---

# Decouple roder-tui's library build from roder-app-server

`roder-tui` no longer depends on the heavy `roder-app-server` crate for its
library build — only on `roder-app-server-core` (`roder-app-server` is now a
dev-dependency, used by tests). This lets the TUI type-check in parallel with
the `server.rs` translation unit instead of serially after it; a `--timings`
rebuild shows the `roder-tui` unit now overlapping `roder-app-server` rather
than starting only once it finishes.

To achieve this, the remote-pairing panel is abstracted behind a new
`roder_tui::RemotePanelHost` trait (with `RemotePanelSnapshot`); the concrete
`AppServer`-backed implementation moved to `roder-cli`
(`remote_panel::AppServerRemotePanel` + `build_tui_app`).

**Breaking:**
- `TuiApp` no longer has a default client type parameter (`TuiApp<C = LocalAppClient>` → `TuiApp<C>`).
- `TuiApp::new` / `TuiApp::new_with_startup` were removed; use `roder_cli::remote_panel::build_tui_app` or `TuiApp::new_with_startup_and_remote` with a `Box<dyn RemotePanelHost>`.
- `TuiApp::new_with_startup_and_remote` now takes `Box<dyn RemotePanelHost>` instead of `Arc<AppServer>`.
- `RemotePanelController` was removed from `roder-tui` (moved to `roder-cli` as `AppServerRemotePanel`).
