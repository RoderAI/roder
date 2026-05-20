use std::sync::Arc;

use roder_api::events::EventEnvelope;
use roder_app_server::AppServer;
use roder_app_server::remote::{
    RemoteServerController, RemoteServerHandle, RemoteServerOptions, RemoteToken,
    generate_remote_token_from_os, listen_remote_websocket_controller,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemotePanelSnapshot {
    pub running: bool,
    pub connect_urls: Vec<String>,
    pub token_preview: Option<String>,
    pub pairing_url: Option<String>,
    pub connected_clients: usize,
    pub tls_warning: Option<String>,
}

pub struct RemotePanelController {
    app_server: Arc<AppServer>,
    listen: String,
    workspace: Option<String>,
    server: Option<RemoteServerController>,
    token: Option<RemoteToken>,
    connected_clients: usize,
}

impl RemotePanelController {
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
            token: None,
            connected_clients: 0,
        }
    }

    pub fn is_running(&self) -> bool {
        self.server.is_some()
    }

    pub fn snapshot(&self) -> RemotePanelSnapshot {
        self.server
            .as_ref()
            .map(|server| RemotePanelSnapshot::from_handle(server.handle(), self.connected_clients))
            .unwrap_or_else(RemotePanelSnapshot::stopped)
    }

    pub async fn start(&mut self) -> anyhow::Result<()> {
        self.start_with_token(generate_remote_token_from_os()?)
            .await
    }

    pub async fn start_with_token(&mut self, token: RemoteToken) -> anyhow::Result<()> {
        if self.server.is_some() {
            self.stop().await?;
        }
        let server = listen_remote_websocket_controller(
            self.app_server.clone(),
            RemoteServerOptions {
                listen: self.listen.clone(),
                token: token.clone(),
                token_ttl: None,
                allowed_origins: Vec::new(),
                print_qr: true,
                workspace: self.workspace.clone(),
            },
        )
        .await?;
        self.server = Some(server);
        self.token = Some(token);
        self.connected_clients = 0;
        Ok(())
    }

    pub async fn regenerate_token(&mut self, token: RemoteToken) -> anyhow::Result<()> {
        self.start_with_token(token).await
    }

    pub async fn stop(&mut self) -> anyhow::Result<()> {
        if let Some(server) = self.server.take() {
            server.stop().await?;
        }
        self.token = None;
        self.connected_clients = 0;
        Ok(())
    }

    pub fn copy_url(&self) -> Option<String> {
        self.snapshot().connect_urls.into_iter().next()
    }

    pub fn copy_auth_header(&self) -> Option<String> {
        self.token
            .as_ref()
            .map(|token| format!("Authorization: Bearer {}", token.secret()))
    }

    pub fn apply_event(&mut self, envelope: &EventEnvelope) {
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
}

impl RemotePanelSnapshot {
    pub fn stopped() -> Self {
        Self {
            running: false,
            connect_urls: Vec::new(),
            token_preview: None,
            pairing_url: None,
            connected_clients: 0,
            tls_warning: None,
        }
    }

    pub fn from_handle(handle: &RemoteServerHandle, connected_clients: usize) -> Self {
        let mut connect_urls = handle.connect_urls.clone();
        connect_urls.sort_by_key(|url| (remote_url_rank(url), url.clone()));
        Self {
            running: true,
            connect_urls,
            token_preview: Some(handle.token_preview.clone()),
            pairing_url: Some(handle.pairing_url.clone()),
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
}

pub fn render_remote_panel_lines(snapshot: &RemotePanelSnapshot) -> Vec<String> {
    if !snapshot.running {
        return vec![
            "Remote app-server: stopped".to_string(),
            "Start remote mode to pair a phone or another local client.".to_string(),
        ];
    }

    let mut lines = vec![
        "Remote app-server: running".to_string(),
        format!("Connected clients: {}", snapshot.connected_clients),
    ];
    if let Some(preview) = snapshot.token_preview.as_ref() {
        lines.push(format!("Token: {preview}"));
    }
    lines.extend(
        snapshot
            .connect_urls
            .iter()
            .map(|url| format!("URL: {url}")),
    );
    if let Some(pairing_url) = snapshot.pairing_url.as_ref() {
        lines.push(format!("Pairing: {pairing_url}"));
    }
    if let Some(warning) = snapshot.tls_warning.as_ref() {
        lines.push(format!("Warning: {warning}"));
    }
    lines
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
        };
        let snapshot = RemotePanelSnapshot::from_handle(&handle, 2);
        let rendered = render_remote_panel_lines(&snapshot).join("\n");

        assert!(rendered.contains("Remote app-server: running"));
        assert!(rendered.contains("Connected clients: 2"));
        assert!(rendered.contains("secr...oken"));
        assert!(rendered.contains("without TLS"));
        assert!(!rendered.contains("secret-token"));
    }

    #[test]
    fn stopped_remote_panel_has_clear_status() {
        let rendered = render_remote_panel_lines(&RemotePanelSnapshot::stopped()).join("\n");

        assert!(rendered.contains("Remote app-server: stopped"));
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
        };
        let snapshot = RemotePanelSnapshot::from_handle(&handle, 0);

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
        let mut controller = RemotePanelController::with_listen(
            app_server(),
            "ws://127.0.0.1:0".to_string(),
            Some("/tmp/gode".to_string()),
        );

        controller
            .start_with_token(RemoteToken::new("first-secret-token".to_string()).unwrap())
            .await
            .unwrap();
        assert!(controller.is_running());
        let first_url = controller.copy_url().expect("running controller has url");
        assert!(first_url.starts_with("ws://127.0.0.1:"));
        assert_eq!(
            controller.copy_auth_header().as_deref(),
            Some("Authorization: Bearer first-secret-token")
        );

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
            .regenerate_token(RemoteToken::new("second-secret-token".to_string()).unwrap())
            .await
            .unwrap();
        assert_eq!(
            controller.copy_auth_header().as_deref(),
            Some("Authorization: Bearer second-secret-token")
        );
        assert_eq!(controller.snapshot().connected_clients, 0);

        let stopped_addr = controller
            .copy_url()
            .expect("running controller has url")
            .strip_prefix("ws://")
            .expect("websocket url")
            .to_string();
        controller.stop().await.unwrap();

        assert!(!controller.is_running());
        assert!(controller.copy_url().is_none());
        assert!(controller.copy_auth_header().is_none());
        assert_eq!(controller.snapshot(), RemotePanelSnapshot::stopped());
        assert!(tokio::net::TcpStream::connect(stopped_addr).await.is_err());
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
