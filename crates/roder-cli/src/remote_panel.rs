//! `AppServer`-backed implementation of `roder_tui::RemotePanelHost`.
//!
//! `roder-tui` defines the `RemotePanelHost` trait so its library can build
//! against `roder-app-server-core` only (in parallel with the heavy server
//! crate). The binary owns the concrete remote-pairing host, which drives a real
//! `roder-app-server` `AppServer` over the WebSocket transport in
//! `roder_app_server::remote`.

use std::sync::Arc;

use async_trait::async_trait;
use roder_api::events::EventEnvelope;
use roder_app_server::AppServer;
use roder_app_server::remote::{
    RemoteServerController, RemoteServerHandle, RemoteServerOptions, RemoteToken,
    generate_remote_token_from_os, listen_remote_websocket_controller, render_pairing_qr,
};
use roder_app_server::LocalAppClient;
use roder_tui::{RemotePanelHost, RemotePanelSnapshot, TuiApp, TuiStartup};

/// Build a `TuiApp` wired to an `AppServer`-backed remote-pairing host derived
/// from the local client. Replaces the old `TuiApp::new_with_startup`
/// convenience constructor that lived in `roder-tui`.
pub async fn build_tui_app(
    client: LocalAppClient,
    model: String,
    startup: TuiStartup,
) -> anyhow::Result<TuiApp<LocalAppClient>> {
    let panel = Box::new(AppServerRemotePanel::new(client.app_server(), cwd_workspace()));
    TuiApp::new_with_startup_and_remote(client, model, startup, panel).await
}

/// Box an `AppServer`-backed remote-pairing host for an explicit server handle.
pub fn remote_panel_for(app_server: Arc<AppServer>) -> Box<dyn RemotePanelHost> {
    Box::new(AppServerRemotePanel::new(app_server, cwd_workspace()))
}

fn cwd_workspace() -> Option<String> {
    std::env::current_dir()
        .ok()
        .map(|path| path.display().to_string())
}

pub struct AppServerRemotePanel {
    app_server: Arc<AppServer>,
    listen: String,
    workspace: Option<String>,
    server: Option<RemoteServerController>,
    connected_clients: usize,
}

impl AppServerRemotePanel {
    pub fn new(app_server: Arc<AppServer>, workspace: Option<String>) -> Self {
        Self::with_listen(app_server, "ws://0.0.0.0:0".to_string(), workspace)
    }

    pub fn with_listen(
        app_server: Arc<AppServer>,
        listen: String,
        workspace: Option<String>,
    ) -> Self {
        Self {
            app_server,
            listen,
            workspace,
            server: None,
            connected_clients: 0,
        }
    }

    pub async fn start_with_token(&mut self, token: RemoteToken) -> anyhow::Result<()> {
        if self.server.is_some() {
            self.stop().await?;
        }
        let server = listen_remote_websocket_controller(
            self.app_server.clone(),
            RemoteServerOptions {
                listen: self.listen.clone(),
                token,
                token_ttl: None,
                allowed_origins: Vec::new(),
                print_qr: true,
                workspace: self.workspace.clone(),
            },
        )
        .await?;
        self.server = Some(server);
        self.connected_clients = 0;
        Ok(())
    }
}

#[async_trait]
impl RemotePanelHost for AppServerRemotePanel {
    fn is_running(&self) -> bool {
        self.server.is_some()
    }

    fn snapshot(&self) -> RemotePanelSnapshot {
        self.server
            .as_ref()
            .map(|server| snapshot_from_handle(server.handle(), self.connected_clients))
            .unwrap_or_else(RemotePanelSnapshot::stopped)
    }

    fn apply_event(&mut self, envelope: &EventEnvelope) {
        match envelope.kind.as_str() {
            "remote/clientConnected" if self.server.is_some() => {
                self.connected_clients = self.connected_clients.saturating_add(1);
            }
            "remote/clientDisconnected" if self.server.is_some() => {
                self.connected_clients = self.connected_clients.saturating_sub(1);
            }
            _ => {}
        }
    }

    async fn start(&mut self) -> anyhow::Result<()> {
        self.start_with_token(generate_remote_token_from_os()?)
            .await
    }

    async fn stop(&mut self) -> anyhow::Result<()> {
        if let Some(server) = self.server.take() {
            server.stop().await?;
        }
        self.connected_clients = 0;
        Ok(())
    }
}

fn snapshot_from_handle(handle: &RemoteServerHandle, connected_clients: usize) -> RemotePanelSnapshot {
    let mut connect_urls = handle.connect_urls.clone();
    connect_urls.sort_by_key(|url| (remote_url_rank(url), url.clone()));
    RemotePanelSnapshot {
        running: true,
        connect_urls,
        token_preview: Some(handle.token_preview.clone()),
        pairing_url: Some(handle.pairing_url.clone()),
        pair_url: Some(handle.pair_url.clone()),
        pairing_qr: render_pairing_qr(&handle.pairing_url).ok(),
        connected_clients,
        tls_warning: handle
            .connect_urls
            .iter()
            .any(|url| !url.starts_with("ws://127.") && !url.starts_with("ws://[::1]"))
            .then_some(
                "LAN remote app-server uses bearer auth without TLS; prefer Tailscale or a trusted network."
                    .to_string(),
            ),
    }
}

fn remote_url_rank(url: &str) -> u8 {
    if url
        .strip_prefix("ws://100.")
        .and_then(|rest| rest.split('.').next())
        .and_then(|octet| octet.parse::<u8>().ok())
        .is_some_and(|octet| (64..=127).contains(&octet))
    {
        0
    } else if url.starts_with("ws://127.") || url.starts_with("ws://[::1]") {
        2
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::events::{
        EventSource, RemoteClientConnected, RemoteClientDisconnected, RoderEvent,
    };
    use roder_core::Runtime;
    use roder_tui::render_remote_panel_lines;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use time::OffsetDateTime;

    fn app_server() -> Arc<AppServer> {
        Arc::new(AppServer::new(Arc::new(
            Runtime::fake().expect("fake runtime"),
        )))
    }

    #[test]
    fn remote_panel_redacts_full_token_and_warns_for_lan_url() {
        let handle = RemoteServerHandle {
            listen_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 4545),
            connect_urls: vec!["ws://192.168.1.20:4545".to_string()],
            token_preview: "secr...oken".to_string(),
            pairing_url: "gode://connect?payload=redacted-fixture".to_string(),
            pair_url: "http://127.0.0.1:4545/pair#roder-pair=redacted".to_string(),
        };
        let snapshot = snapshot_from_handle(&handle, 2);
        let rendered = render_remote_panel_lines(&snapshot).join("\n");

        assert!(rendered.contains("Remote app-server: running"));
        assert!(rendered.contains("Connected clients: 2"));
        assert!(rendered.contains("secr...oken"));
        assert!(rendered.contains("without TLS"));
        assert!(!rendered.contains("secret-token"));
    }

    #[test]
    fn remote_panel_prefers_tailscale_urls_before_lan_and_loopback() {
        let handle = RemoteServerHandle {
            listen_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 4545),
            connect_urls: vec![
                "ws://127.0.0.1:4545".to_string(),
                "ws://192.168.1.20:4545".to_string(),
                "ws://100.90.80.70:4545".to_string(),
            ],
            token_preview: "secr...oken".to_string(),
            pairing_url: "gode://connect?payload=redacted-fixture".to_string(),
            pair_url: "http://127.0.0.1:4545/pair#roder-pair=redacted".to_string(),
        };
        let snapshot = snapshot_from_handle(&handle, 0);

        assert_eq!(
            snapshot.connect_urls,
            vec![
                "ws://100.90.80.70:4545",
                "ws://192.168.1.20:4545",
                "ws://127.0.0.1:4545",
            ]
        );
    }

    #[tokio::test]
    async fn remote_panel_controller_starts_stops_regenerates_and_copies_pairing_values() {
        let mut controller = AppServerRemotePanel::with_listen(
            app_server(),
            "ws://127.0.0.1:0".to_string(),
            Some("/tmp/gode".to_string()),
        );

        controller
            .start_with_token(RemoteToken::new("first-secret-token".to_string()).unwrap())
            .await
            .unwrap();
        assert!(controller.is_running());
        let first_url = controller
            .snapshot()
            .connect_urls
            .into_iter()
            .next()
            .expect("running controller has url");
        assert!(first_url.starts_with("ws://127.0.0.1:"));

        controller.apply_event(&remote_event(RoderEvent::RemoteClientConnected(
            RemoteClientConnected {
                remote_addr: Some("127.0.0.1:50000".to_string()),
                timestamp: OffsetDateTime::now_utc(),
            },
        )));
        let rendered = render_remote_panel_lines(&controller.snapshot()).join("\n");
        assert!(rendered.contains("Connected clients: 1"));
        assert!(rendered.contains("URL: ws://127.0.0.1:"));
        assert!(rendered.contains("Pairing: roder://connect?payload="));
        assert!(rendered.contains("Token: firs...oken"));
        controller.apply_event(&remote_event(RoderEvent::RemoteClientDisconnected(
            RemoteClientDisconnected {
                remote_addr: Some("127.0.0.1:50000".to_string()),
                timestamp: OffsetDateTime::now_utc(),
            },
        )));
        assert_eq!(controller.snapshot().connected_clients, 0);

        controller
            .start_with_token(RemoteToken::new("second-secret-token".to_string()).unwrap())
            .await
            .unwrap();
        assert_eq!(controller.snapshot().connected_clients, 0);

        controller.stop().await.unwrap();

        assert!(!controller.is_running());
        assert_eq!(controller.snapshot(), RemotePanelSnapshot::stopped());
    }

    fn remote_event(event: RoderEvent) -> EventEnvelope {
        EventEnvelope {
            event_id: "event-test".to_string(),
            seq: 1,
            timestamp: OffsetDateTime::now_utc(),
            source: EventSource::AppServer,
            kind: event.kind().to_string(),
            thread_id: None,
            turn_id: None,
            event,
        }
    }
}
