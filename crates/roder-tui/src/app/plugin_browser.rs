use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use roder_api::marketplace::{
    DedupedMarketplacePlugin, MarketplacePluginRisk, MarketplacePluginVariant, variant_key,
};
use roder_protocol::{
    JsonRpcRequest, MarketplacesSearchParams, MarketplacesSearchResult,
    PluginInstallAllVariantsParams, PluginInstallAllVariantsResult, PluginInstallParams,
    PluginInstallResult,
};

use super::{Theme, TuiApp, decode_response, truncate};

#[derive(Debug, Clone, Eq, PartialEq)]
pub(super) enum PluginBrowserStatus {
    Ready,
    Empty,
    Error(String),
    Installed(String),
    ConfirmInstallAll(String),
}

#[derive(Debug, Clone)]
pub(super) struct PluginBrowserState {
    plugins: Vec<DedupedMarketplacePlugin>,
    list_state: ListState,
    status: PluginBrowserStatus,
    pending_install_all_key: Option<String>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(super) enum PluginBrowserAction {
    Close,
    Handled,
    InstallSelected,
    InstallAllVariants,
}

impl PluginBrowserState {
    pub(super) fn new(plugins: Vec<DedupedMarketplacePlugin>) -> Self {
        let mut list_state = ListState::default();
        list_state.select((!plugins.is_empty()).then_some(0));
        let status = if plugins.is_empty() {
            PluginBrowserStatus::Empty
        } else {
            PluginBrowserStatus::Ready
        };
        Self {
            plugins,
            list_state,
            status,
            pending_install_all_key: None,
        }
    }

    pub(super) fn with_error(message: impl Into<String>) -> Self {
        Self {
            plugins: Vec::new(),
            list_state: ListState::default(),
            status: PluginBrowserStatus::Error(message.into()),
            pending_install_all_key: None,
        }
    }

    fn selected_plugin(&self) -> Option<&DedupedMarketplacePlugin> {
        self.list_state
            .selected()
            .and_then(|index| self.plugins.get(index))
    }

    fn selected_install_variant(&self) -> Option<&MarketplacePluginVariant> {
        let plugin = self.selected_plugin()?;
        plugin
            .variants
            .iter()
            .find(|variant| {
                !plugin
                    .installed_variants
                    .contains(&variant_key_for(variant))
            })
            .or_else(|| plugin.variants.first())
    }

    pub(super) fn selected_install_target(&self) -> Option<(String, String)> {
        let variant = self.selected_install_variant()?;
        Some((variant.marketplace_id.clone(), variant.plugin_id.clone()))
    }

    pub(super) fn selected_plugin_target(&self) -> Option<(String, String)> {
        let plugin = self.selected_plugin()?;
        let variant = plugin.variants.first()?;
        Some((variant.marketplace_id.clone(), variant.plugin_id.clone()))
    }

    pub(super) fn set_installed(&mut self, label: impl Into<String>) {
        self.status = PluginBrowserStatus::Installed(label.into());
    }

    pub(super) fn set_error(&mut self, message: impl Into<String>) {
        self.status = PluginBrowserStatus::Error(message.into());
    }

    pub(super) fn handle_key(&mut self, key: KeyEvent) -> PluginBrowserAction {
        match key.code {
            KeyCode::Esc => PluginBrowserAction::Close,
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_selection(-1);
                self.pending_install_all_key = None;
                PluginBrowserAction::Handled
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_selection(1);
                self.pending_install_all_key = None;
                PluginBrowserAction::Handled
            }
            KeyCode::PageUp => {
                self.move_selection(-10);
                self.pending_install_all_key = None;
                PluginBrowserAction::Handled
            }
            KeyCode::PageDown => {
                self.move_selection(10);
                self.pending_install_all_key = None;
                PluginBrowserAction::Handled
            }
            KeyCode::Home => {
                if !self.plugins.is_empty() {
                    self.list_state.select(Some(0));
                }
                self.pending_install_all_key = None;
                PluginBrowserAction::Handled
            }
            KeyCode::End => {
                if !self.plugins.is_empty() {
                    self.list_state.select(Some(self.plugins.len() - 1));
                }
                self.pending_install_all_key = None;
                PluginBrowserAction::Handled
            }
            KeyCode::Enter => PluginBrowserAction::InstallSelected,
            KeyCode::Char('a') | KeyCode::Char('A') if key.modifiers == KeyModifiers::NONE => {
                self.confirm_or_request_install_all()
            }
            _ => PluginBrowserAction::Handled,
        }
    }

    fn confirm_or_request_install_all(&mut self) -> PluginBrowserAction {
        let Some((marketplace_id, plugin_id)) = self.selected_plugin_target() else {
            return PluginBrowserAction::Handled;
        };
        let key = variant_key(&marketplace_id, &plugin_id);
        if self.pending_install_all_key.as_deref() == Some(&key) {
            self.pending_install_all_key = None;
            return PluginBrowserAction::InstallAllVariants;
        }
        self.pending_install_all_key = Some(key);
        self.status = PluginBrowserStatus::ConfirmInstallAll(
            "Press A again to install all variants; duplicate skills, commands, hooks, and MCP servers may appear with provider namespaces."
                .to_string(),
        );
        PluginBrowserAction::Handled
    }

    fn move_selection(&mut self, delta: isize) {
        if self.plugins.is_empty() {
            self.list_state.select(None);
            return;
        }
        let current = self.list_state.selected().unwrap_or(0);
        let next = (current as isize + delta)
            .clamp(0, self.plugins.len().saturating_sub(1) as isize) as usize;
        self.list_state.select(Some(next));
    }
}

impl TuiApp {
    pub(super) async fn open_plugin_browser(&mut self) {
        self.show_provider_popup = false;
        self.plugin_browser = Some(match self.marketplaces_search(None).await {
            Ok(result) => PluginBrowserState::new(result.plugins),
            Err(err) => {
                PluginBrowserState::with_error(format!("marketplaces/search failed: {err}"))
            }
        });
    }

    pub(super) async fn handle_plugin_browser_key(&mut self, key: KeyEvent) {
        let Some(action) = self
            .plugin_browser
            .as_mut()
            .map(|browser| browser.handle_key(key))
        else {
            return;
        };
        match action {
            PluginBrowserAction::Close => self.plugin_browser = None,
            PluginBrowserAction::Handled => {}
            PluginBrowserAction::InstallSelected => self.install_selected_browser_plugin().await,
            PluginBrowserAction::InstallAllVariants => {
                self.install_all_browser_plugin_variants().await;
            }
        }
    }

    async fn install_selected_browser_plugin(&mut self) {
        let Some((marketplace_id, plugin_id)) = self
            .plugin_browser
            .as_ref()
            .and_then(PluginBrowserState::selected_install_target)
        else {
            return;
        };
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("plugins/install")),
                method: "plugins/install".to_string(),
                params: Some(
                    serde_json::to_value(PluginInstallParams {
                        marketplace_id: marketplace_id.clone(),
                        plugin_id: plugin_id.clone(),
                    })
                    .unwrap(),
                ),
            })
            .await;
        match decode_response::<PluginInstallResult>(res) {
            Ok(result) => {
                if let Some(browser) = self.plugin_browser.as_mut() {
                    browser.set_installed(format!("installed {}", result.plugin.variant_key));
                }
                self.push_event(format!("plugin installed: {}", result.plugin.variant_key));
                self.refresh_plugin_browser().await;
            }
            Err(err) => {
                if let Some(browser) = self.plugin_browser.as_mut() {
                    browser.set_error(format!("install failed: {err}"));
                }
            }
        }
    }

    async fn install_all_browser_plugin_variants(&mut self) {
        let Some((marketplace_id, plugin_id)) = self
            .plugin_browser
            .as_ref()
            .and_then(PluginBrowserState::selected_plugin_target)
        else {
            return;
        };
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("plugins/install_all_variants")),
                method: "plugins/install_all_variants".to_string(),
                params: Some(
                    serde_json::to_value(PluginInstallAllVariantsParams {
                        marketplace_id,
                        plugin_id,
                    })
                    .unwrap(),
                ),
            })
            .await;
        match decode_response::<PluginInstallAllVariantsResult>(res) {
            Ok(result) => {
                let count = result.plugins.len();
                if let Some(browser) = self.plugin_browser.as_mut() {
                    browser.set_installed(format!("installed {count} variants"));
                }
                self.push_event(format!("plugin variants installed: {count}"));
                self.refresh_plugin_browser().await;
            }
            Err(err) => {
                if let Some(browser) = self.plugin_browser.as_mut() {
                    browser.set_error(format!("install all failed: {err}"));
                }
            }
        }
    }

    async fn refresh_plugin_browser(&mut self) {
        let selected = self
            .plugin_browser
            .as_ref()
            .and_then(|browser| browser.list_state.selected())
            .unwrap_or(0);
        let status = self
            .plugin_browser
            .as_ref()
            .map(|browser| browser.status.clone())
            .unwrap_or(PluginBrowserStatus::Ready);
        if let Ok(result) = self.marketplaces_search(None).await {
            let mut browser = PluginBrowserState::new(result.plugins);
            if !browser.plugins.is_empty() {
                browser
                    .list_state
                    .select(Some(selected.min(browser.plugins.len() - 1)));
            }
            browser.status = status;
            browser.pending_install_all_key = None;
            self.plugin_browser = Some(browser);
        }
    }

    async fn marketplaces_search(
        &self,
        query: Option<String>,
    ) -> anyhow::Result<MarketplacesSearchResult> {
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("marketplaces/search")),
                method: "marketplaces/search".to_string(),
                params: Some(serde_json::to_value(MarketplacesSearchParams { query })?),
            })
            .await;
        decode_response(res)
    }

    pub(super) fn render_plugin_browser(&mut self, f: &mut Frame<'_>, area: Rect) {
        let Some(browser) = self.plugin_browser.as_mut() else {
            return;
        };
        render_plugin_browser(f, area, browser, self.theme);
    }
}

pub(super) fn render_plugin_browser(
    f: &mut Frame<'_>,
    area: Rect,
    state: &mut PluginBrowserState,
    theme: Theme,
) {
    let dialog_area = ninety_percent_rect(area);
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
            " Install plugins (Enter install, A install all variants, Esc close) ",
            theme.accent(),
        ));
    let inner = block.inner(dialog_area);
    f.render_widget(block, dialog_area);

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(32),
            Constraint::Length(1),
            Constraint::Percentage(67),
        ])
        .split(inner);
    render_plugin_list(f, columns[0], state, theme);
    render_plugin_details(f, columns[2], state, theme);
}

fn render_plugin_list(f: &mut Frame<'_>, area: Rect, state: &mut PluginBrowserState, theme: Theme) {
    let block = panel_block(" Plugins ", theme);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let items = if state.plugins.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "No installable plugins found",
            theme.muted(),
        )))]
    } else {
        state
            .plugins
            .iter()
            .map(|plugin| {
                let width = inner.width.saturating_sub(4).max(8) as usize;
                let installed = if plugin.installed_variants.is_empty() {
                    " "
                } else {
                    "✓"
                };
                ListItem::new(vec![
                    Line::from(vec![
                        Span::styled(format!("{installed} "), theme.subtle()),
                        Span::styled(truncate(&plugin.display_name, width), theme.text()),
                    ]),
                    Line::from(Span::styled(
                        format!(
                            "{} variants · {}",
                            plugin.variants.len(),
                            plugin.identity_key.canonical_slug
                        ),
                        theme.muted(),
                    )),
                ])
            })
            .collect()
    };

    let list = List::new(items)
        .style(theme.dialog_surface())
        .highlight_style(theme.selected())
        .highlight_symbol("› ");
    f.render_stateful_widget(list, inner, &mut state.list_state);
}

fn render_plugin_details(f: &mut Frame<'_>, area: Rect, state: &PluginBrowserState, theme: Theme) {
    let block = panel_block(" Details ", theme);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(8),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);
    let detail = match state.selected_plugin() {
        Some(plugin) => plugin_detail_text(plugin, theme),
        None => Text::from(Line::from(Span::styled(
            status_text(&state.status),
            theme.muted(),
        ))),
    };
    f.render_widget(
        Paragraph::new(detail)
            .style(theme.dialog_surface())
            .wrap(Wrap { trim: true }),
        chunks[0],
    );
    f.render_widget(status_line(&state.status, theme), chunks[1]);
    f.render_widget(action_line(state.selected_plugin(), theme), chunks[2]);
}

fn panel_block(title: &'static str, theme: Theme) -> Block<'static> {
    let borders = if theme.borders_visible {
        Borders::ALL
    } else {
        Borders::NONE
    };
    Block::default()
        .borders(borders)
        .border_type(theme.border_type)
        .border_style(theme.dialog())
        .style(theme.dialog_surface())
        .title(Span::styled(title, theme.accent_soft()))
}

fn plugin_detail_text(plugin: &DedupedMarketplacePlugin, theme: Theme) -> Text<'static> {
    let mut lines = vec![
        Line::from(Span::styled(plugin.display_name.clone(), theme.strong())),
        Line::from(Span::styled(
            plugin
                .description
                .clone()
                .unwrap_or_else(|| "No description provided.".to_string()),
            theme.text(),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("Identity ", theme.accent_soft()),
            Span::styled(plugin.identity_key.canonical_slug.clone(), theme.muted()),
        ]),
        Line::from(vec![
            Span::styled("Installed variants ", theme.accent_soft()),
            Span::styled(plugin.installed_variants.len().to_string(), theme.muted()),
        ]),
        Line::from(vec![
            Span::styled("Recommended ", theme.accent_soft()),
            Span::styled(
                plugin
                    .recommended_variant_key
                    .clone()
                    .unwrap_or_else(|| "none".to_string()),
                theme.muted(),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled("Variants", theme.accent())),
    ];
    lines.extend(plugin.variants.iter().map(|variant| {
        let key = variant_key_for(variant);
        let installed = plugin.installed_variants.contains(&key);
        Line::from(vec![
            Span::styled(if installed { "✓ " } else { "• " }, theme.subtle()),
            Span::styled(format!("{:?}", variant.kind), theme.text()),
            Span::styled(format!("  {}", variant.marketplace_id), theme.muted()),
            Span::styled(
                format!("  {}", risk_label(&variant.risk)),
                risk_style(&variant.risk, theme),
            ),
            Span::styled(format!("  {}", variant.plugin_id), theme.subtle()),
        ])
    }));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Selected preview", theme.accent())));
    if let Some(variant) = plugin.variants.first() {
        lines.push(Line::from(vec![
            Span::styled("Source ", theme.accent_soft()),
            Span::styled(source_label(&variant.source), theme.muted()),
        ]));
        lines.push(Line::from(vec![
            Span::styled("Components ", theme.accent_soft()),
            Span::styled(component_label(variant), theme.muted()),
        ]));
        lines.push(Line::from(vec![
            Span::styled("Capabilities ", theme.accent_soft()),
            Span::styled(capability_label(variant), theme.muted()),
        ]));
        lines.push(Line::from(vec![
            Span::styled("Posture ", theme.accent_soft()),
            Span::styled(risk_label(&variant.risk), risk_style(&variant.risk, theme)),
        ]));
    }
    if !plugin.related_candidates.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Related candidates",
            theme.accent(),
        )));
        lines.extend(plugin.related_candidates.iter().map(|variant| {
            Line::from(vec![
                Span::styled("• ", theme.subtle()),
                Span::styled(format!("{:?}", variant.kind), theme.text()),
                Span::styled(format!("  {}", variant.marketplace_id), theme.muted()),
                Span::styled(format!("  {}", variant.plugin_id), theme.subtle()),
            ])
        }));
    }
    Text::from(lines)
}

fn status_line(status: &PluginBrowserStatus, theme: Theme) -> Paragraph<'static> {
    let style = match status {
        PluginBrowserStatus::Error(_) => theme.error(),
        PluginBrowserStatus::ConfirmInstallAll(_) => theme.error(),
        PluginBrowserStatus::Installed(_) => theme.accent_soft(),
        PluginBrowserStatus::Ready | PluginBrowserStatus::Empty => theme.muted(),
    };
    Paragraph::new(Line::from(Span::styled(status_text(status), style)))
        .style(theme.dialog_surface())
}

fn status_text(status: &PluginBrowserStatus) -> String {
    match status {
        PluginBrowserStatus::Ready => {
            "Choose a plugin. Enter installs the first available variant.".to_string()
        }
        PluginBrowserStatus::Empty => {
            "No plugins found. Install or refresh marketplaces first.".to_string()
        }
        PluginBrowserStatus::Error(message) => message.clone(),
        PluginBrowserStatus::Installed(message) => message.clone(),
        PluginBrowserStatus::ConfirmInstallAll(message) => message.clone(),
    }
}

fn action_line(plugin: Option<&DedupedMarketplacePlugin>, theme: Theme) -> Paragraph<'static> {
    let install_label = if plugin.is_some_and(|plugin| !plugin.installed_variants.is_empty()) {
        " Reinstall selected "
    } else {
        " Install now "
    };
    Paragraph::new(Line::from(vec![
        chip(install_label, plugin.is_some(), theme),
        Span::styled("  Enter", theme.dialog_key()),
        Span::styled(" selected variant   ", theme.muted()),
        Span::styled("A", theme.dialog_key()),
        Span::styled(" all variants   ", theme.muted()),
        Span::styled("Esc", theme.dialog_key()),
        Span::styled(" close", theme.muted()),
    ]))
    .style(theme.dialog_surface())
}

fn chip(label: &'static str, enabled: bool, theme: Theme) -> Span<'static> {
    if enabled {
        Span::styled(label, theme.dialog_key())
    } else {
        Span::styled(label, theme.muted())
    }
}

fn risk_label(risk: &MarketplacePluginRisk) -> &'static str {
    match risk {
        MarketplacePluginRisk::Passive => "passive",
        MarketplacePluginRisk::ReadsWorkspace => "reads workspace",
        MarketplacePluginRisk::StartsProcess => "starts process",
        MarketplacePluginRisk::RunsHook => "runs hook",
        MarketplacePluginRisk::Unknown => "unknown risk",
    }
}

fn risk_style(risk: &MarketplacePluginRisk, theme: Theme) -> ratatui::style::Style {
    match risk {
        MarketplacePluginRisk::Passive => theme.muted(),
        MarketplacePluginRisk::ReadsWorkspace => theme.shell(),
        MarketplacePluginRisk::StartsProcess
        | MarketplacePluginRisk::RunsHook
        | MarketplacePluginRisk::Unknown => theme.error(),
    }
}

fn source_label(source: &roder_api::marketplace::PluginSource) -> String {
    match source {
        roder_api::marketplace::PluginSource::MarketplacePath {
            marketplace_id,
            path,
        } => format!("{marketplace_id}/{path}"),
        roder_api::marketplace::PluginSource::Git { url, path, .. } => path
            .as_ref()
            .map(|path| format!("{url}:{path}"))
            .unwrap_or_else(|| url.clone()),
        roder_api::marketplace::PluginSource::Http { url, .. } => url.clone(),
        roder_api::marketplace::PluginSource::Npm { package, version } => version
            .as_ref()
            .map(|version| format!("{package}@{version}"))
            .unwrap_or_else(|| package.clone()),
        roder_api::marketplace::PluginSource::LocalPath { path } => path.clone(),
        roder_api::marketplace::PluginSource::Unsupported { .. } => "unsupported".to_string(),
    }
}

fn component_label(variant: &MarketplacePluginVariant) -> String {
    let hints = &variant.component_hints;
    let mut labels = Vec::new();
    if hints.skills {
        labels.push("skills");
    }
    if hints.commands {
        labels.push("commands");
    }
    if hints.agents {
        labels.push("agents");
    }
    if hints.mcp_servers {
        labels.push("mcp");
    }
    if hints.hooks {
        labels.push("hooks");
    }
    if hints.apps {
        labels.push("apps");
    }
    if hints.lsp_servers {
        labels.push("lsp");
    }
    if hints.rules {
        labels.push("rules");
    }
    if hints.assets {
        labels.push("assets");
    }
    if labels.is_empty() {
        "none".to_string()
    } else {
        labels.join(", ")
    }
}

fn capability_label(variant: &MarketplacePluginVariant) -> String {
    if variant.capability_hints.is_empty() {
        "none".to_string()
    } else {
        variant.capability_hints.join(", ")
    }
}

fn variant_key_for(variant: &MarketplacePluginVariant) -> String {
    variant_key(&variant.marketplace_id, &variant.plugin_id)
}

fn ninety_percent_rect(area: Rect) -> Rect {
    let width = ((area.width as u32 * 90) / 100).max(40) as u16;
    let height = ((area.height as u32 * 90) / 100).max(12) as u16;
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
    use roder_api::marketplace::{
        MarketplaceKind, PluginIdentityKey, PluginSource, normalize_slug,
    };

    #[test]
    fn state_selects_first_plugin() {
        let state = PluginBrowserState::new(vec![sample_plugin()]);
        assert_eq!(state.list_state.selected(), Some(0));
        assert_eq!(
            state.selected_install_target(),
            Some(("claude-local".to_string(), "repo-tools".to_string()))
        );
    }

    #[test]
    fn keymap_installs_with_enter_and_closes_with_escape() {
        let mut state = PluginBrowserState::new(vec![sample_plugin()]);
        assert_eq!(
            state.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            PluginBrowserAction::InstallSelected
        );
        assert_eq!(
            state.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)),
            PluginBrowserAction::Handled
        );
        assert_eq!(
            state.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)),
            PluginBrowserAction::InstallAllVariants
        );
        assert_eq!(
            state.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            PluginBrowserAction::Close
        );
    }

    #[test]
    fn dialog_area_uses_most_of_available_space() {
        assert_eq!(ninety_percent_rect(Rect::new(0, 0, 100, 40)).width, 90);
        assert_eq!(ninety_percent_rect(Rect::new(0, 0, 100, 40)).height, 36);
    }

    #[test]
    fn detail_text_includes_preview_posture_and_variant_metadata() {
        let text = plugin_detail_text(&sample_plugin(), Theme::for_terminal());
        let rendered = text
            .lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<Vec<_>>()
            .join("");

        assert!(rendered.contains("Selected preview"));
        assert!(rendered.contains("Source "));
        assert!(rendered.contains("Components "));
        assert!(rendered.contains("mcp"));
        assert!(rendered.contains("Recommended "));
    }

    fn sample_plugin() -> DedupedMarketplacePlugin {
        DedupedMarketplacePlugin {
            identity_key: PluginIdentityKey {
                canonical_slug: normalize_slug("github.com/example/repo-tools"),
                normalized_name: "repo-tools".to_string(),
                repository: Some("https://github.com/example/repo-tools".to_string()),
                homepage_domain: Some("github.com".to_string()),
                author_name: None,
            },
            display_name: "Repo Tools".to_string(),
            description: Some("Repository helper skills".to_string()),
            variants: vec![MarketplacePluginVariant {
                marketplace_id: "claude-local".to_string(),
                plugin_id: "repo-tools".to_string(),
                kind: MarketplaceKind::Claude,
                source: PluginSource::MarketplacePath {
                    marketplace_id: "claude-local".to_string(),
                    path: "repo-tools".to_string(),
                },
                homepage: Some("https://github.com/example/repo-tools".to_string()),
                category: None,
                tags: Vec::new(),
                component_hints: roder_api::marketplace::PluginComponentHints {
                    mcp_servers: true,
                    ..Default::default()
                },
                capability_hints: Vec::new(),
                version: None,
                content_hash: None,
                risk: MarketplacePluginRisk::Passive,
            }],
            related_candidates: Vec::new(),
            recommended_variant_key: Some("claude-local:repo-tools".to_string()),
            installed_variants: Vec::new(),
        }
    }
}
