use async_trait::async_trait;
use roder_api::events::EventEnvelope;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemotePanelSnapshot {
    pub running: bool,
    pub connect_urls: Vec<String>,
    pub token_preview: Option<String>,
    pub pairing_url: Option<String>,
    pub pair_url: Option<String>,
    pub pairing_qr: Option<String>,
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
            pair_url: None,
            pairing_qr: None,
            connected_clients: 0,
            tls_warning: None,
        }
    }
}

/// Drives the remote-pairing app-server from inside the TUI without binding the
/// TUI to the concrete `roder-app-server` `AppServer`/`remote` types. The
/// binary (`roder-cli`) supplies the `AppServer`-backed implementation; tests
/// supply a lightweight fake. This indirection lets `roder-tui` compile against
/// `roder-app-server-core` only, in parallel with the heavy server crate.
#[async_trait]
pub trait RemotePanelHost: Send {
    fn is_running(&self) -> bool;
    fn snapshot(&self) -> RemotePanelSnapshot;
    fn apply_event(&mut self, envelope: &EventEnvelope);
    async fn start(&mut self) -> anyhow::Result<()>;
    async fn stop(&mut self) -> anyhow::Result<()>;
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
    if let Some(pair_url) = snapshot.pair_url.as_ref() {
        lines.push(format!("pair: {pair_url}"));
    }
    if let Some(pairing_qr) = snapshot.pairing_qr.as_ref() {
        lines.push("QR:".to_string());
        lines.extend(pairing_qr.lines().map(ToString::to_string));
    }
    if let Some(warning) = snapshot.tls_warning.as_ref() {
        lines.push(format!("Warning: {warning}"));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopped_remote_panel_has_clear_status() {
        let rendered = render_remote_panel_lines(&RemotePanelSnapshot::stopped()).join("\n");

        assert!(rendered.contains("Remote app-server: stopped"));
    }
}
