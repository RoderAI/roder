use roder_app_server::remote::RemoteServerHandle;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemotePanelSnapshot {
    pub running: bool,
    pub connect_urls: Vec<String>,
    pub token_preview: Option<String>,
    pub pairing_url: Option<String>,
    pub connected_clients: usize,
    pub tls_warning: Option<String>,
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
        Self {
            running: true,
            connect_urls: handle.connect_urls.clone(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

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
}
