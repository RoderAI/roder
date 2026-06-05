use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use roder_api::chrome::{ChromePermissionMode, ChromeStatus, ChromeTab};
use roder_protocol::JsonRpcRequest;
use serde_json::json;

use super::remote::render_remote_panel_lines;
use super::{AppClient, Theme, TuiApp, decode_response, truncate};

/// Result of a fetched chrome tab listing, kept small so the panel only holds
/// display-ready, untrusted strings (never re-interpreted as commands).
#[derive(Debug, Clone, Default)]
pub(super) struct ChromeTabsView {
    pub(super) count: usize,
    pub(super) titles: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct ChromePanelState {
    status: Option<ChromeStatus>,
    tabs: Option<ChromeTabsView>,
    last_error: Option<String>,
    selected: usize,
}

impl ChromePanelState {
    pub(super) fn new(status: Option<ChromeStatus>, last_error: Option<String>) -> Self {
        Self {
            status,
            tabs: None,
            last_error,
            selected: 0,
        }
    }
}

/// Advance the permission mode in a stable observe → assist → control → observe
/// cycle. Pulled out so it can be unit-tested without an app client.
pub(super) fn next_chrome_mode(mode: ChromePermissionMode) -> ChromePermissionMode {
    match mode {
        ChromePermissionMode::Observe => ChromePermissionMode::Assist,
        ChromePermissionMode::Assist => ChromePermissionMode::Control,
        ChromePermissionMode::Control => ChromePermissionMode::Observe,
    }
}

impl<C> TuiApp<C>
where
    C: AppClient,
{
    pub(super) async fn open_chrome_panel(&mut self) {
        self.show_provider_popup = false;
        let state = match self.chrome_status().await {
            Ok(status) => ChromePanelState::new(Some(status), None),
            Err(err) => ChromePanelState::new(None, Some(format!("chrome/status failed: {err}"))),
        };
        self.chrome_panel = Some(state);
    }

    /// One-click pairing from the ctrl+p palette: open the panel, start the
    /// remote app-server, and open the auto-pair URL in the browser so the
    /// loaded extension configures and connects itself.
    pub(super) async fn chrome_pair_oneclick(&mut self) {
        self.open_chrome_panel().await;
        match self.remote_panel.start().await {
            Ok(()) => match self.remote_panel.snapshot().pair_url {
                Some(url) => {
                    open_url_in_browser(&url);
                    self.set_chrome_error(None);
                }
                None => self.set_chrome_error(Some(
                    "pairing started but no pair URL was available".to_string(),
                )),
            },
            Err(err) => self.set_chrome_error(Some(format!("pairing start failed: {err}"))),
        }
        self.refresh_chrome_panel().await;
    }

    pub(super) async fn handle_chrome_panel_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.chrome_panel = None;
            }
            KeyCode::Char('p') => {
                match self.remote_panel.start().await {
                    Ok(()) => self.set_chrome_error(None),
                    Err(err) => self.set_chrome_error(Some(format!("pairing start failed: {err}"))),
                }
                self.refresh_chrome_panel().await;
            }
            KeyCode::Char('e') => {
                let mode = self
                    .chrome_panel
                    .as_ref()
                    .and_then(|panel| panel.status.as_ref())
                    .map(|status| status.mode)
                    .unwrap_or_default();
                self.chrome_action("chrome/enable", Some(json!({ "mode": mode.as_str() })))
                    .await;
            }
            KeyCode::Char('d') => {
                self.chrome_action("chrome/disable", None).await;
            }
            KeyCode::Char('m') => {
                let next = self
                    .chrome_panel
                    .as_ref()
                    .and_then(|panel| panel.status.as_ref())
                    .map(|status| next_chrome_mode(status.mode))
                    .unwrap_or_default();
                self.chrome_action("chrome/setMode", Some(json!({ "mode": next.as_str() })))
                    .await;
            }
            KeyCode::Char('r') => {
                self.chrome_action("chrome/reconnect", None).await;
            }
            KeyCode::Char('t') => {
                self.chrome_list_tabs().await;
            }
            KeyCode::Char('o') => match self.remote_panel.snapshot().pair_url {
                Some(url) => {
                    open_url_in_browser(&url);
                    self.set_chrome_error(None);
                }
                None => self.set_chrome_error(Some(
                    "no pair URL yet — press p to start pairing first".to_string(),
                )),
            },
            _ => {}
        }
    }

    /// Call a `chrome/*` method that returns a [`ChromeStatus`], store it, and
    /// re-fetch status so the panel stays consistent.
    async fn chrome_action(&mut self, method: &str, params: Option<serde_json::Value>) {
        match self.chrome_request(method, params).await {
            Ok(status) => {
                if let Some(panel) = self.chrome_panel.as_mut() {
                    panel.status = Some(status);
                    panel.last_error = None;
                }
            }
            Err(err) => self.set_chrome_error(Some(format!("{method} failed: {err}"))),
        }
        self.refresh_chrome_panel().await;
    }

    async fn chrome_list_tabs(&mut self) {
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(json!("chrome/tabs/list")),
                method: "chrome/tabs/list".to_string(),
                params: None,
            })
            .await;
        match decode_response::<ChromeTabsResult>(res) {
            Ok(result) => {
                let titles = result.tabs.iter().map(chrome_tab_label).take(12).collect();
                if let Some(panel) = self.chrome_panel.as_mut() {
                    panel.tabs = Some(ChromeTabsView {
                        count: result.tabs.len(),
                        titles,
                    });
                    panel.last_error = None;
                }
            }
            Err(err) => self.set_chrome_error(Some(format!("chrome/tabs/list failed: {err}"))),
        }
        self.refresh_chrome_panel().await;
    }

    async fn refresh_chrome_panel(&mut self) {
        if self.chrome_panel.is_none() {
            return;
        }
        match self.chrome_status().await {
            Ok(status) => {
                if let Some(panel) = self.chrome_panel.as_mut() {
                    panel.status = Some(status);
                }
            }
            Err(err) => self.set_chrome_error(Some(format!("chrome/status failed: {err}"))),
        }
    }

    fn set_chrome_error(&mut self, error: Option<String>) {
        if let Some(panel) = self.chrome_panel.as_mut() {
            panel.last_error = error;
        }
    }

    async fn chrome_status(&self) -> anyhow::Result<ChromeStatus> {
        self.chrome_request("chrome/status", None).await
    }

    async fn chrome_request(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> anyhow::Result<ChromeStatus> {
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(json!(method)),
                method: method.to_string(),
                params,
            })
            .await;
        decode_response(res)
    }

    pub(super) fn render_chrome_panel(&mut self, f: &mut Frame<'_>, area: Rect) {
        let Some(panel) = self.chrome_panel.as_ref() else {
            return;
        };
        let snapshot = self.remote_panel.snapshot();
        render_chrome_panel(f, area, panel, &snapshot, self.theme);
    }
}

/// The `/chrome` slash-command surface. Mirrors `/remote`: a small set of verbs
/// that act and then print to the timeline, plus opening the modal.
impl<C> TuiApp<C>
where
    C: AppClient,
{
    pub(super) async fn run_chrome_slash_command(&mut self, args: &str) {
        let action = args.split_whitespace().next().unwrap_or("");
        match action {
            "" | "panel" => {
                self.open_chrome_panel().await;
                self.push_event("slash command: /chrome panel".to_string());
            }
            "status" => {
                self.print_chrome_status().await;
            }
            "enable" => {
                let mode = ChromePermissionMode::Assist;
                match self
                    .chrome_request("chrome/enable", Some(json!({ "mode": mode.as_str() })))
                    .await
                {
                    Ok(status) => {
                        self.timeline.push_system(chrome_status_summary(&status));
                        self.push_event("slash command: /chrome enable".to_string());
                    }
                    Err(err) => self.record_error(format!("chrome enable failed: {err}")),
                }
            }
            "disable" => match self.chrome_request("chrome/disable", None).await {
                Ok(status) => {
                    self.timeline.push_system(chrome_status_summary(&status));
                    self.push_event("slash command: /chrome disable".to_string());
                }
                Err(err) => self.record_error(format!("chrome disable failed: {err}")),
            },
            "reconnect" => match self.chrome_request("chrome/reconnect", None).await {
                Ok(status) => {
                    self.timeline.push_system(chrome_status_summary(&status));
                    self.push_event("slash command: /chrome reconnect".to_string());
                }
                Err(err) => self.record_error(format!("chrome reconnect failed: {err}")),
            },
            "pair" => match self.remote_panel.start().await {
                Ok(()) => {
                    let snapshot = self.remote_panel.snapshot();
                    if let Some(pair_url) = snapshot.pair_url.as_ref() {
                        self.timeline
                            .push_system(format!("1-click pair URL: {pair_url}"));
                    }
                    self.timeline
                        .push_system(render_remote_panel_lines(&snapshot).join("\n"));
                    self.push_event("slash command: /chrome pair".to_string());
                }
                Err(err) => self.record_error(format!("chrome pair failed: {err}")),
            },
            other => {
                self.timeline.push_error(format!(
                    "unknown /chrome action: {other}. Use status, enable, disable, reconnect, pair, or panel."
                ));
            }
        }
    }

    async fn print_chrome_status(&mut self) {
        match self.chrome_status().await {
            Ok(status) => {
                self.timeline.push_system(chrome_status_summary(&status));
                self.push_event("slash command: /chrome status".to_string());
            }
            Err(err) => self.record_error(format!("chrome status failed: {err}")),
        }
    }
}

#[derive(Debug, serde::Deserialize)]
struct ChromeTabsResult {
    #[serde(default)]
    tabs: Vec<ChromeTab>,
}

/// Render a single tab as plain, untrusted display text. Titles and URLs come
/// from the browser, so they are truncated and never treated as commands.
fn chrome_tab_label(tab: &ChromeTab) -> String {
    let title = tab
        .title
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("(untitled)");
    let url = tab.url.as_deref().unwrap_or("");
    let label = if url.is_empty() {
        title.to_string()
    } else {
        format!("{title} — {url}")
    };
    truncate(&label, 90)
}

fn chrome_status_summary(status: &ChromeStatus) -> String {
    let mut lines = vec![
        format!(
            "Chrome: {}",
            if status.connected {
                "connected"
            } else {
                "not connected"
            }
        ),
        format!(
            "Clients: {}  ·  enabled: {}  ·  mode: {}",
            status.client_count,
            if status.enabled { "yes" } else { "no" },
            status.mode.as_str()
        ),
    ];
    if let Some(addr) = status.remote_addr.as_ref() {
        lines.push(format!("Remote: {addr}"));
    }
    if !status.capabilities.is_empty() {
        lines.push(format!("Capabilities: {}", status.capabilities.join(", ")));
    }
    if let Some(tab) = status.active_tab.as_ref() {
        lines.push(format!("Active tab: {}", chrome_tab_label(tab)));
    }
    if let Some(err) = status.last_error.as_ref() {
        lines.push(format!("Last error: {err}"));
    }
    lines.join("\n")
}

/// Open `url` in the user's default browser via the OS opener. Spawned without
/// waiting so the TUI never blocks; output is ignored.
fn open_url_in_browser(url: &str) {
    use std::process::Command;
    let mut command = if cfg!(target_os = "macos") {
        let mut command = Command::new("open");
        command.arg(url);
        command
    } else {
        let mut command = Command::new("xdg-open");
        command.arg(url);
        command
    };
    let _ = command.spawn();
}

fn render_chrome_panel(
    f: &mut Frame<'_>,
    area: Rect,
    state: &ChromePanelState,
    remote: &super::remote::RemotePanelSnapshot,
    theme: Theme,
) {
    let dialog_area = chrome_dialog_rect(area);
    f.render_widget(Clear, dialog_area);

    let borders = if theme.borders_visible {
        Borders::ALL
    } else {
        Borders::NONE
    };
    let block = Block::default()
        .borders(borders)
        .border_type(theme.border_type)
        .border_style(theme.dialog())
        .style(theme.dialog_surface())
        .title(Span::styled(
            " Chrome browser plugin (p pair · o open · e enable · d disable · m mode · r reconnect · t tabs · Esc close) ",
            theme.accent(),
        ));
    let inner = block.inner(dialog_area);
    f.render_widget(block, dialog_area);

    let text = Text::from(chrome_panel_lines(state, remote, theme));
    f.render_widget(
        Paragraph::new(text)
            .style(theme.dialog_surface())
            .wrap(Wrap { trim: false }),
        inner,
    );
}

fn chrome_panel_lines<'a>(
    state: &ChromePanelState,
    remote: &super::remote::RemotePanelSnapshot,
    theme: Theme,
) -> Vec<Line<'a>> {
    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled("Connection", theme.accent())));
    match state.status.as_ref() {
        Some(status) => {
            lines.push(kv(
                "Connected",
                if status.connected { "yes" } else { "no" },
                theme,
            ));
            lines.push(kv("Clients", &status.client_count.to_string(), theme));
            if let Some(addr) = status.remote_addr.as_ref() {
                lines.push(kv("Remote addr", addr, theme));
            }
            if let Some(browser) = status.browser.as_ref() {
                lines.push(kv(
                    "Browser",
                    &format!("{} ({})", browser.name, browser.kind),
                    theme,
                ));
            }
            lines.push(kv(
                "Enabled",
                if status.enabled { "yes" } else { "no" },
                theme,
            ));
            lines.push(kv("Mode", status.mode.as_str(), theme));
            let caps = if status.capabilities.is_empty() {
                "none".to_string()
            } else {
                status.capabilities.join(", ")
            };
            lines.push(kv("Capabilities", &caps, theme));
            match status.active_tab.as_ref() {
                Some(tab) => lines.push(kv("Active tab", &chrome_tab_label(tab), theme)),
                None => lines.push(kv("Active tab", "none", theme)),
            }
        }
        None => {
            lines.push(Line::from(Span::styled(
                "Status unavailable.",
                theme.muted(),
            )));
        }
    }

    if let Some(tabs) = state.tabs.as_ref() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("Tabs ({})", tabs.count),
            theme.accent(),
        )));
        if tabs.titles.is_empty() {
            lines.push(Line::from(Span::styled("(no tabs)", theme.muted())));
        } else {
            for title in &tabs.titles {
                lines.push(Line::from(vec![
                    Span::styled("• ", theme.subtle()),
                    Span::styled(title.clone(), theme.text()),
                ]));
            }
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Pairing", theme.accent())));
    if let Some(pair_url) = remote.pair_url.as_ref() {
        lines.push(Line::from(vec![
            Span::styled("1-click pair URL (press o): ", theme.accent_soft()),
            Span::styled(pair_url.clone(), theme.text()),
        ]));
    }
    for line in render_remote_panel_lines(remote) {
        lines.push(Line::from(Span::styled(line, theme.text())));
    }

    if let Some(err) = state.last_error.as_ref() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(err.clone(), theme.error())));
    }
    let _ = state.selected;
    lines
}

fn kv<'a>(key: &str, value: &str, theme: Theme) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("{key}: "), theme.accent_soft()),
        Span::styled(value.to_string(), theme.text()),
    ])
}

fn chrome_dialog_rect(area: Rect) -> Rect {
    let width = ((area.width as u32 * 80) / 100).max(40) as u16;
    let height = ((area.height as u32 * 80) / 100).max(12) as u16;
    Rect {
        x: area.x + area.width.saturating_sub(width.min(area.width)) / 2,
        y: area.y + area.height.saturating_sub(height.min(area.height)) / 2,
        width: width.min(area.width),
        height: height.min(area.height),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_cycles_observe_assist_control() {
        assert_eq!(
            next_chrome_mode(ChromePermissionMode::Observe),
            ChromePermissionMode::Assist
        );
        assert_eq!(
            next_chrome_mode(ChromePermissionMode::Assist),
            ChromePermissionMode::Control
        );
        assert_eq!(
            next_chrome_mode(ChromePermissionMode::Control),
            ChromePermissionMode::Observe
        );
    }

    #[test]
    fn tab_label_is_plain_truncated_text() {
        let tab = ChromeTab {
            id: 1,
            window_id: None,
            title: Some("Example".to_string()),
            url: Some("https://example.com".to_string()),
            fav_icon_url: None,
            active: true,
        };
        let label = chrome_tab_label(&tab);
        assert!(label.contains("Example"));
        assert!(label.contains("https://example.com"));
    }

    #[test]
    fn tab_label_falls_back_for_blank_title() {
        let tab = ChromeTab {
            id: 1,
            window_id: None,
            title: Some("   ".to_string()),
            url: None,
            fav_icon_url: None,
            active: false,
        };
        assert_eq!(chrome_tab_label(&tab), "(untitled)");
    }

    #[test]
    fn status_summary_includes_mode_and_connection() {
        let status = ChromeStatus {
            connected: true,
            client_count: 2,
            enabled: true,
            capabilities: vec!["tabs.list".to_string()],
            mode: ChromePermissionMode::Control,
            ..Default::default()
        };
        let summary = chrome_status_summary(&status);
        assert!(summary.contains("connected"));
        assert!(summary.contains("control"));
        assert!(summary.contains("tabs.list"));
    }
}
