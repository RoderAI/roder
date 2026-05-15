use std::io;
use std::time::Duration;

use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph},
};
use roder_api::events::RoderEvent;
use roder_app_server::LocalAppClient;
use roder_protocol::{
    CodexAuthResult, CreateSessionResult, InterruptTurnParams, JsonRpcRequest, JsonRpcResponse,
    ProviderSelectParams, ProviderSelectResult, ProvidersListResult, StartTurnParams,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Theme {
    text: Color,
    text_strong: Color,
    muted: Color,
    subtle: Color,
    accent: Color,
    accent_soft: Color,
    tool: Color,
    tool_running: Color,
    error: Color,
    border: Color,
    dialog: Color,
    selection_fg: Color,
    selection_bg: Color,
    top_bar_track: Color,
    top_bar_fill: Color,
}

impl Theme {
    fn for_terminal() -> Self {
        Self::for_dark_background(detect_dark_background())
    }

    fn for_dark_background(dark: bool) -> Self {
        if dark {
            return Self {
                text: Color::Reset,
                text_strong: Color::Reset,
                muted: Color::Indexed(244),
                subtle: Color::Indexed(245),
                accent: Color::Indexed(212),
                accent_soft: Color::Indexed(183),
                tool: Color::Indexed(214),
                tool_running: Color::Indexed(75),
                error: Color::Indexed(196),
                border: Color::Indexed(244),
                dialog: Color::Indexed(62),
                selection_fg: Color::Reset,
                selection_bg: Color::Indexed(212),
                top_bar_track: Color::Indexed(236),
                top_bar_fill: Color::Indexed(212),
            };
        }

        Self {
            text: Color::Reset,
            text_strong: Color::Reset,
            muted: Color::Indexed(240),
            subtle: Color::Indexed(240),
            accent: Color::Indexed(198),
            accent_soft: Color::Indexed(96),
            tool: Color::Indexed(172),
            tool_running: Color::Indexed(25),
            error: Color::Indexed(160),
            border: Color::Indexed(240),
            dialog: Color::Indexed(62),
            selection_fg: Color::Reset,
            selection_bg: Color::Indexed(198),
            top_bar_track: Color::Indexed(252),
            top_bar_fill: Color::Indexed(198),
        }
    }

    fn text(self) -> Style {
        Style::default().fg(self.text)
    }

    fn strong(self) -> Style {
        Style::default()
            .fg(self.text_strong)
            .add_modifier(Modifier::BOLD)
    }

    fn muted(self) -> Style {
        Style::default().fg(self.muted)
    }

    fn subtle(self) -> Style {
        Style::default().fg(self.subtle)
    }

    fn accent(self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    fn accent_soft(self) -> Style {
        Style::default()
            .fg(self.accent_soft)
            .add_modifier(Modifier::BOLD)
    }

    fn tool(self) -> Style {
        Style::default().fg(self.tool).add_modifier(Modifier::BOLD)
    }

    fn running(self) -> Style {
        Style::default()
            .fg(self.tool_running)
            .add_modifier(Modifier::BOLD)
    }

    fn error(self) -> Style {
        Style::default().fg(self.error).add_modifier(Modifier::BOLD)
    }

    fn border(self) -> Style {
        Style::default().fg(self.border)
    }

    fn dialog(self) -> Style {
        Style::default().fg(self.dialog)
    }

    fn selected(self) -> Style {
        Style::default().fg(self.selection_fg).bg(self.selection_bg)
    }
}

#[derive(Debug, Clone)]
struct ProviderOption {
    provider_id: String,
    model_id: String,
    label: String,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum ProviderPopupScreen {
    Main,
    Models,
}

#[derive(Debug, Clone)]
enum ProviderMenuItem {
    Models,
    Model(ProviderOption),
    CodexLogin,
    CodexStatus,
    CodexLogout,
    Back,
}

impl ProviderMenuItem {
    fn label(&self) -> String {
        match self {
            Self::Models => "Models".to_string(),
            Self::Model(option) => option.label.clone(),
            Self::CodexLogin => "Sign in with Codex".to_string(),
            Self::CodexStatus => "Codex auth status".to_string(),
            Self::CodexLogout => "Sign out of Codex".to_string(),
            Self::Back => "Back".to_string(),
        }
    }
}

pub struct TuiApp {
    client: LocalAppClient,
    thread_id: String,
    active_turn_id: Option<String>,
    provider: String,
    model: String,
    input: String,
    messages: Vec<String>,
    events: Vec<String>,
    animation_frame: u64,
    show_event_log: bool,
    show_provider_popup: bool,
    provider_popup_screen: ProviderPopupScreen,
    model_options: Vec<ProviderOption>,
    provider_menu_items: Vec<ProviderMenuItem>,
    provider_state: ListState,
    theme: Theme,
}

impl TuiApp {
    pub async fn new(client: LocalAppClient, model: String) -> anyhow::Result<Self> {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(1)),
            method: "sessions/create".to_string(),
            params: None,
        };

        let res = client.send_request(req).await;
        let session = if let Some(result) = res.result {
            serde_json::from_value::<CreateSessionResult>(result)?
        } else {
            anyhow::bail!("failed to create session: {:?}", res.error);
        };

        let mut provider_state = ListState::default();
        provider_state.select(Some(0));

        Ok(Self {
            client,
            thread_id: session.thread_id,
            active_turn_id: None,
            provider: session.provider,
            model: if model.is_empty() {
                session.model
            } else {
                model
            },
            input: String::new(),
            messages: Vec::new(),
            events: Vec::new(),
            animation_frame: 0,
            show_event_log: false,
            show_provider_popup: false,
            provider_popup_screen: ProviderPopupScreen::Main,
            model_options: Vec::new(),
            provider_menu_items: Vec::new(),
            provider_state,
            theme: Theme::for_terminal(),
        })
    }

    pub async fn run(&mut self) -> anyhow::Result<()> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let mut rx = self.client.subscribe_events();

        loop {
            self.animation_frame = self.animation_frame.wrapping_add(1);
            terminal.draw(|f| self.render(f))?;

            if event::poll(Duration::from_millis(50))?
                && let Event::Key(key) = event::read()?
            {
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('p') {
                    if self.show_provider_popup {
                        self.show_provider_popup = false;
                    } else {
                        self.open_provider_popup().await;
                    }
                } else if key.modifiers.contains(KeyModifiers::CONTROL)
                    && key.code == KeyCode::Char('l')
                {
                    self.show_event_log = !self.show_event_log;
                    self.push_event(if self.show_event_log {
                        "event log shown".to_string()
                    } else {
                        "event log hidden".to_string()
                    });
                } else if key.modifiers.contains(KeyModifiers::CONTROL)
                    && key.code == KeyCode::Char('c')
                {
                    if let Some(turn_id) = self.active_turn_id.clone() {
                        let client = self.client.clone();
                        let thread_id = self.thread_id.clone();
                        tokio::spawn(async move {
                            let params = InterruptTurnParams { thread_id, turn_id };
                            let _ = client
                                .send_request(JsonRpcRequest {
                                    jsonrpc: "2.0".to_string(),
                                    id: Some(serde_json::json!("interrupt")),
                                    method: "turns/interrupt".to_string(),
                                    params: Some(serde_json::to_value(params).unwrap()),
                                })
                                .await;
                        });
                    }
                } else if self.show_provider_popup {
                    match key.code {
                        KeyCode::Esc => self.close_or_back_provider_popup(),
                        KeyCode::Up => self.select_previous_provider_menu_item(),
                        KeyCode::Down => self.select_next_provider_menu_item(),
                        KeyCode::Enter => self.select_current_provider_menu_item().await,
                        _ => {}
                    }
                } else {
                    match key.code {
                        KeyCode::Char(c) => self.input.push(c),
                        KeyCode::Backspace => {
                            self.input.pop();
                        }
                        KeyCode::Enter => {
                            let text = self.input.trim().to_string();
                            self.input.clear();
                            if text.is_empty() {
                                continue;
                            }
                            if text.starts_with('/') {
                                self.messages
                                    .push(format!("executed slash command: {text}"));
                                continue;
                            }
                            self.messages.push(format!("user: {text}"));
                            let params = StartTurnParams {
                                thread_id: self.thread_id.clone(),
                                message: text,
                                provider_override: None,
                                model_override: Some(self.model.clone()),
                            };
                            let client = self.client.clone();
                            tokio::spawn(async move {
                                let _ = client
                                    .send_request(JsonRpcRequest {
                                        jsonrpc: "2.0".to_string(),
                                        id: Some(serde_json::json!("turns/start")),
                                        method: "turns/start".to_string(),
                                        params: Some(serde_json::to_value(params).unwrap()),
                                    })
                                    .await;
                            });
                        }
                        KeyCode::Esc => break,
                        _ => {}
                    }
                }
            }

            while let Ok(envelope) = rx.try_recv() {
                self.push_event(format!("{} #{}", envelope.kind, envelope.seq));

                match envelope.event {
                    RoderEvent::TurnStarted(ev) => self.active_turn_id = Some(ev.turn_id),
                    RoderEvent::TurnCompleted(ev)
                        if self.active_turn_id.as_deref() == Some(&ev.turn_id) =>
                    {
                        self.active_turn_id = None;
                    }
                    RoderEvent::TurnInterrupted(ev)
                        if self.active_turn_id.as_deref() == Some(&ev.turn_id) =>
                    {
                        self.active_turn_id = None;
                    }
                    RoderEvent::InferenceEventReceived(ev) => {
                        if let roder_api::inference::InferenceEvent::MessageDelta(delta) = ev.event
                        {
                            if let Some(last) = self.messages.last_mut()
                                && last.starts_with("assistant: ")
                            {
                                last.push_str(&delta.text);
                                continue;
                            }
                            self.messages.push(format!("assistant: {}", delta.text));
                        }
                    }
                    RoderEvent::TurnFailed(ev) => {
                        self.messages.push(format!("error: {}", ev.error))
                    }
                    _ => {}
                }
            }
        }

        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
        terminal.show_cursor()?;

        Ok(())
    }

    fn render(&mut self, f: &mut Frame<'_>) {
        let area = f.area();
        let event_height = event_log_height(self.show_event_log, self.events.len());
        let mut constraints = vec![
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(5),
        ];
        if event_height > 0 {
            constraints.push(Constraint::Length(event_height));
        }
        constraints.extend([Constraint::Length(3), Constraint::Length(1)]);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(area);

        f.render_widget(self.animated_top_bar(area.width), chunks[0]);
        f.render_widget(self.header(area.width), chunks[1]);
        f.render_widget(self.transcript(), chunks[2]);

        let composer_index = if event_height > 0 {
            f.render_widget(self.event_log(), chunks[3]);
            4
        } else {
            3
        };
        f.render_widget(self.composer(), chunks[composer_index]);
        f.render_widget(self.footer(area.width), chunks[composer_index + 1]);

        if self.show_provider_popup {
            self.render_provider_popup(f, area);
        }
    }

    fn animated_top_bar(&self, width: u16) -> Paragraph<'static> {
        Paragraph::new(animated_bar_line(
            width,
            self.animation_frame,
            self.theme.top_bar_track,
            self.theme.top_bar_fill,
        ))
    }

    fn header(&self, width: u16) -> Paragraph<'static> {
        let model_label = format!("{}/{}", self.provider, self.model);
        let left = vec![
            Span::styled(" roder", self.theme.accent()),
            Span::styled(format!("  {model_label}"), self.theme.text()),
            Span::styled(
                format!("  session {}", short_id(&self.thread_id)),
                self.theme.muted(),
            ),
        ];
        let turn = self
            .active_turn_id
            .as_deref()
            .map(short_id)
            .unwrap_or("idle");
        let right_style = if self.active_turn_id.is_some() {
            self.theme.running()
        } else {
            self.theme.muted()
        };
        Paragraph::new(line_with_gap(
            left,
            vec![Span::styled(turn.to_string(), right_style)],
            width,
            self.theme.text(),
        ))
    }

    fn transcript(&self) -> Paragraph<'static> {
        let text = if self.messages.is_empty() {
            Text::from(vec![
                Line::raw(""),
                Line::from(Span::styled(
                    "No transcript yet. Ask Roder to inspect, edit, or run something.",
                    self.theme.muted().add_modifier(Modifier::ITALIC),
                )),
            ])
        } else {
            Text::from(
                self.messages
                    .iter()
                    .flat_map(|message| [message_line(message, self.theme), Line::raw("")])
                    .collect::<Vec<_>>(),
            )
        };

        Paragraph::new(text).style(self.theme.text())
    }

    fn event_log(&self) -> Paragraph<'static> {
        let lines = self
            .events
            .iter()
            .rev()
            .take(6)
            .rev()
            .map(|event| {
                Line::from(vec![
                    Span::styled("• ", self.theme.subtle()),
                    Span::styled(event.to_string(), self.theme.muted()),
                ])
            })
            .collect::<Vec<_>>();

        let text = if lines.is_empty() {
            Text::from(Line::from(Span::styled(
                "No events yet.",
                self.theme.muted().add_modifier(Modifier::ITALIC),
            )))
        } else {
            Text::from(lines)
        };

        Paragraph::new(text).block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(self.theme.border())
                .title(Span::styled(" events ", self.theme.muted())),
        )
    }

    fn composer(&self) -> Paragraph<'static> {
        let text = if self.input.is_empty() {
            Text::from(Line::from(Span::styled(
                "Ask Roder to work on this repo",
                self.theme.muted().add_modifier(Modifier::ITALIC),
            )))
        } else {
            Text::from(Line::from(Span::styled(
                self.input.clone(),
                self.theme.text(),
            )))
        };

        Paragraph::new(text).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(self.theme.border())
                .title(Span::styled(" composer ", self.theme.muted())),
        )
    }

    fn footer(&self, width: u16) -> Paragraph<'static> {
        let status = if self.active_turn_id.is_some() {
            "running"
        } else {
            "ready"
        };
        Paragraph::new(line_with_gap(
            vec![Span::styled(
                format!(
                    " {status}  enter send  ctrl+p provider/model  ctrl+l events  ctrl+c interrupt  esc quit"
                ),
                self.theme.subtle(),
            )],
            vec![Span::styled(
                format!("events {} ", self.events.len()),
                self.theme.muted(),
            )],
            width,
            self.theme.subtle(),
        ))
    }

    fn render_provider_popup(&mut self, f: &mut Frame<'_>, area: Rect) {
        let menu_area = centered_rect(area, area.width.min(72), area.height.min(16));
        let items: Vec<ListItem> = if self.provider_menu_items.is_empty() {
            vec![ListItem::new(Line::from(Span::styled(
                "No menu items available",
                self.theme.muted(),
            )))]
        } else {
            self.provider_menu_items
                .iter()
                .map(|item| {
                    ListItem::new(Line::from(vec![
                        Span::styled("• ", self.theme.subtle()),
                        Span::styled(item.label(), self.theme.text()),
                    ]))
                })
                .collect()
        };
        let title = match self.provider_popup_screen {
            ProviderPopupScreen::Main => " Provider (Enter select, Esc close) ",
            ProviderPopupScreen::Models => " Models (Enter select, Esc back) ",
        };
        let menu = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(self.theme.dialog())
                    .title(Span::styled(title, self.theme.accent())),
            )
            .highlight_style(self.theme.selected())
            .highlight_symbol("› ");
        f.render_widget(Clear, menu_area);
        f.render_stateful_widget(menu, menu_area, &mut self.provider_state);
    }

    async fn open_provider_popup(&mut self) {
        match self.providers_list().await {
            Ok(list) => {
                self.provider = list.active_provider.clone();
                self.model = list.active_model.clone();
                self.model_options = provider_options_from_list(&list);
                self.provider_popup_screen = ProviderPopupScreen::Main;
                self.provider_menu_items = main_provider_menu_items();
                if self.provider_menu_items.is_empty() {
                    self.provider_state.select(None);
                } else {
                    self.provider_state.select(Some(0));
                }
                self.show_provider_popup = true;
            }
            Err(err) => {
                self.show_provider_popup = false;
                self.record_error(format!("providers/list failed: {err}"));
            }
        }
    }

    async fn providers_list(&self) -> anyhow::Result<ProvidersListResult> {
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("providers/list")),
                method: "providers/list".to_string(),
                params: None,
            })
            .await;
        decode_response(res)
    }

    fn close_or_back_provider_popup(&mut self) {
        if self.provider_popup_screen == ProviderPopupScreen::Models {
            self.provider_popup_screen = ProviderPopupScreen::Main;
            self.provider_menu_items = main_provider_menu_items();
            self.provider_state.select(Some(0));
        } else {
            self.show_provider_popup = false;
        }
    }

    fn select_previous_provider_menu_item(&mut self) {
        if self.provider_menu_items.is_empty() {
            return;
        }
        let last = self.provider_menu_items.len() - 1;
        let i = match self.provider_state.selected() {
            Some(0) | None => last,
            Some(i) => i - 1,
        };
        self.provider_state.select(Some(i));
    }

    fn select_next_provider_menu_item(&mut self) {
        if self.provider_menu_items.is_empty() {
            return;
        }
        let last = self.provider_menu_items.len() - 1;
        let i = match self.provider_state.selected() {
            Some(i) if i >= last => 0,
            Some(i) => i + 1,
            None => 0,
        };
        self.provider_state.select(Some(i));
    }

    async fn select_current_provider_menu_item(&mut self) {
        let Some(selected) = self.provider_state.selected() else {
            self.show_provider_popup = false;
            return;
        };
        let Some(item) = self.provider_menu_items.get(selected).cloned() else {
            self.show_provider_popup = false;
            return;
        };

        match item {
            ProviderMenuItem::Models => {
                self.open_models_submenu();
            }
            ProviderMenuItem::Model(option) => {
                self.select_provider_model(option).await;
            }
            ProviderMenuItem::CodexLogin => {
                self.run_codex_auth("auth/codex/login").await;
            }
            ProviderMenuItem::CodexStatus => {
                self.run_codex_auth("auth/codex/status").await;
            }
            ProviderMenuItem::CodexLogout => {
                self.run_codex_auth("auth/codex/logout").await;
            }
            ProviderMenuItem::Back => {
                self.provider_popup_screen = ProviderPopupScreen::Main;
                self.provider_menu_items = main_provider_menu_items();
                self.provider_state.select(Some(0));
            }
        }
    }

    fn open_models_submenu(&mut self) {
        self.provider_popup_screen = ProviderPopupScreen::Models;
        self.provider_menu_items = self
            .model_options
            .iter()
            .cloned()
            .map(ProviderMenuItem::Model)
            .chain(std::iter::once(ProviderMenuItem::Back))
            .collect();
        let selected = self
            .model_options
            .iter()
            .position(|option| option.provider_id == self.provider && option.model_id == self.model)
            .unwrap_or(0);
        if self.provider_menu_items.is_empty() {
            self.provider_state.select(None);
        } else {
            self.provider_state.select(Some(selected));
        }
    }

    async fn select_provider_model(&mut self, option: ProviderOption) {
        let params = ProviderSelectParams {
            provider: option.provider_id,
            model: Some(option.model_id),
        };
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("providers/select")),
                method: "providers/select".to_string(),
                params: Some(serde_json::to_value(params).unwrap()),
            })
            .await;

        match decode_response::<ProviderSelectResult>(res) {
            Ok(selected) => {
                self.provider = selected.provider;
                self.model = selected.model;
                self.messages.push(format!(
                    "system: switched provider/model to {}/{}.",
                    self.provider, self.model
                ));
                self.push_event(format!(
                    "provider selected: {}/{}",
                    self.provider, self.model
                ));
                self.show_provider_popup = false;
            }
            Err(err) => {
                self.record_error(format!("providers/select failed: {err}"));
                self.show_provider_popup = false;
            }
        }
    }

    async fn run_codex_auth(&mut self, method: &str) {
        if method == "auth/codex/login" {
            self.messages
                .push("system: opening browser for Codex sign-in.".to_string());
        }
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!(method)),
                method: method.to_string(),
                params: None,
            })
            .await;
        match decode_response::<CodexAuthResult>(res) {
            Ok(result) => {
                self.messages.push(codex_auth_message(method, &result));
                self.push_event(format!("codex auth: {}", codex_auth_event(&result)));
                self.show_provider_popup = false;
            }
            Err(err) => {
                self.record_error(format!("codex auth failed: {err}"));
                self.show_provider_popup = false;
            }
        }
    }

    fn record_error(&mut self, message: String) {
        self.messages.push(format!("error: {message}"));
        self.push_event(format!("error: {message}"));
    }

    fn push_event(&mut self, event: String) {
        self.events.push(event);
        if self.events.len() > 12 {
            self.events.remove(0);
        }
    }
}

fn animated_bar_line(width: u16, frame: u64, track: Color, fill: Color) -> Line<'static> {
    let width = usize::from(width);
    if width == 0 {
        return Line::raw("");
    }

    let highlight_width = animated_bar_highlight_width(width);
    let offset = animated_bar_offset(width, highlight_width, frame);
    let mut spans = Vec::new();
    if offset > 0 {
        spans.push(Span::styled(" ".repeat(offset), Style::default().bg(track)));
    }
    spans.push(Span::styled(
        " ".repeat(highlight_width),
        Style::default().bg(fill),
    ));
    let tail = width.saturating_sub(offset + highlight_width);
    if tail > 0 {
        spans.push(Span::styled(" ".repeat(tail), Style::default().bg(track)));
    }
    Line::from(spans)
}

fn animated_bar_highlight_width(width: usize) -> usize {
    (width / 4).clamp(8, 48).min(width)
}

fn animated_bar_offset(width: usize, highlight_width: usize, frame: u64) -> usize {
    let travel = width.saturating_sub(highlight_width);
    if travel == 0 {
        return 0;
    }
    let period = travel * 2;
    let phase = (frame as usize) % period;
    if phase <= travel {
        phase
    } else {
        period - phase
    }
}

fn event_log_height(show_event_log: bool, event_count: usize) -> u16 {
    if show_event_log {
        (event_count as u16 + 2).clamp(3, 8)
    } else {
        0
    }
}

fn provider_options_from_list(list: &ProvidersListResult) -> Vec<ProviderOption> {
    let mut options = Vec::new();
    for provider in &list.providers {
        if provider.models.is_empty() {
            options.push(ProviderOption {
                provider_id: provider.id.clone(),
                model_id: list.active_model.clone(),
                label: format!("{} / {}", provider.id, list.active_model),
            });
            continue;
        }
        for model in &provider.models {
            let model_name = if model.name.is_empty() {
                model.id.clone()
            } else {
                format!("{} ({})", model.id, model.name)
            };
            options.push(ProviderOption {
                provider_id: provider.id.clone(),
                model_id: model.id.clone(),
                label: format!("{} / {}", provider.id, model_name),
            });
        }
    }
    options
}

fn main_provider_menu_items() -> Vec<ProviderMenuItem> {
    vec![
        ProviderMenuItem::Models,
        ProviderMenuItem::CodexLogin,
        ProviderMenuItem::CodexStatus,
        ProviderMenuItem::CodexLogout,
    ]
}

fn codex_auth_message(method: &str, result: &CodexAuthResult) -> String {
    match (method, result.signed_in, result.account_id.as_deref()) {
        ("auth/codex/logout", _, _) => "system: signed out of Codex.".to_string(),
        (_, true, Some(account_id)) => {
            format!("system: signed in with Codex account {account_id}.")
        }
        (_, true, None) => "system: signed in with Codex.".to_string(),
        _ => "system: signed out of Codex.".to_string(),
    }
}

fn codex_auth_event(result: &CodexAuthResult) -> &'static str {
    if result.signed_in {
        "signed in"
    } else {
        "signed out"
    }
}

fn decode_response<T: serde::de::DeserializeOwned>(res: JsonRpcResponse) -> anyhow::Result<T> {
    if let Some(error) = res.error {
        anyhow::bail!("{} ({})", error.message, error.code);
    }
    let Some(result) = res.result else {
        anyhow::bail!("missing result");
    };
    Ok(serde_json::from_value(result)?)
}

fn message_line(message: &str, theme: Theme) -> Line<'static> {
    if let Some(body) = message.strip_prefix("user: ") {
        return Line::from(vec![
            Span::styled("│ ", theme.accent()),
            Span::styled("you ", theme.accent()),
            Span::styled(body.to_string(), theme.text()),
        ]);
    }
    if let Some(body) = message.strip_prefix("assistant: ") {
        return Line::from(vec![
            Span::styled("│ ", theme.accent_soft()),
            Span::styled("roder ", theme.strong()),
            Span::styled(body.to_string(), theme.text()),
        ]);
    }
    if let Some(body) = message.strip_prefix("error: ") {
        return Line::from(vec![
            Span::styled("! ", theme.error()),
            Span::styled(body.to_string(), theme.error()),
        ]);
    }
    if let Some(body) = message.strip_prefix("executed slash command: ") {
        return Line::from(vec![
            Span::styled("/ ", theme.tool()),
            Span::styled(body.to_string(), theme.muted()),
        ]);
    }
    if let Some(body) = message.strip_prefix("system: ") {
        return Line::from(vec![
            Span::styled("• ", theme.subtle()),
            Span::styled(body.to_string(), theme.muted()),
        ]);
    }
    Line::from(Span::styled(message.to_string(), theme.text()))
}

fn line_with_gap(
    mut left: Vec<Span<'static>>,
    right: Vec<Span<'static>>,
    width: u16,
    gap_style: Style,
) -> Line<'static> {
    let left_width = spans_width(&left);
    let right_width = spans_width(&right);
    let gap = usize::from(width)
        .saturating_sub(left_width + right_width)
        .max(1);
    left.push(Span::styled(" ".repeat(gap), gap_style));
    left.extend(right);
    Line::from(left)
}

fn spans_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(|span| span.content.chars().count()).sum()
}

fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width.min(area.width),
        height.min(area.height),
    )
}

fn detect_dark_background() -> bool {
    std::env::var("COLORFGBG")
        .ok()
        .and_then(|value| {
            value
                .rsplit(';')
                .next()
                .and_then(|bg| bg.parse::<u8>().ok())
        })
        .map(|bg| matches!(bg, 0..=6 | 8))
        .unwrap_or(true)
}

fn short_id(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_protocol::{ProviderDescriptor, ProvidersListResult};

    #[test]
    fn theme_primary_text_uses_terminal_default_for_contrast() {
        for dark in [true, false] {
            let theme = Theme::for_dark_background(dark);
            assert_eq!(theme.text, Color::Reset);
            assert_eq!(theme.text_strong, Color::Reset);
        }
    }

    #[test]
    fn semantic_theme_roles_do_not_use_named_black_or_white() {
        for dark in [true, false] {
            let theme = Theme::for_dark_background(dark);
            let colors = [
                theme.text,
                theme.text_strong,
                theme.muted,
                theme.subtle,
                theme.accent,
                theme.accent_soft,
                theme.tool,
                theme.tool_running,
                theme.error,
                theme.border,
                theme.dialog,
                theme.selection_fg,
                theme.selection_bg,
                theme.top_bar_track,
                theme.top_bar_fill,
            ];
            assert!(!colors.contains(&Color::White));
            assert!(!colors.contains(&Color::Black));
        }
    }

    #[test]
    fn line_with_gap_keeps_right_content_at_edge_when_possible() {
        let line = line_with_gap(
            vec![Span::raw("left")],
            vec![Span::raw("right")],
            12,
            Style::default(),
        );
        let rendered = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert_eq!(rendered, "left   right");
    }

    #[test]
    fn message_line_assigns_semantic_prefixes() {
        let line = message_line("assistant: hello", Theme::for_dark_background(true));
        let rendered = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert_eq!(rendered, "│ roder hello");
    }

    #[test]
    fn animated_bar_offset_bounces_between_edges() {
        assert_eq!(animated_bar_offset(20, 5, 0), 0);
        assert_eq!(animated_bar_offset(20, 5, 15), 15);
        assert_eq!(animated_bar_offset(20, 5, 16), 14);
        assert_eq!(animated_bar_offset(20, 5, 30), 0);
    }

    #[test]
    fn animated_bar_highlight_width_stays_within_width() {
        assert_eq!(animated_bar_highlight_width(0), 0);
        assert_eq!(animated_bar_highlight_width(4), 4);
        assert_eq!(animated_bar_highlight_width(80), 20);
        assert_eq!(animated_bar_highlight_width(400), 48);
    }

    #[test]
    fn event_log_height_only_allocates_space_when_toggled_on() {
        assert_eq!(event_log_height(false, 10), 0);
        assert_eq!(event_log_height(true, 0), 3);
        assert_eq!(event_log_height(true, 3), 5);
        assert_eq!(event_log_height(true, 100), 8);
    }

    #[test]
    fn provider_options_include_provider_models() {
        let list = ProvidersListResult {
            active_provider: "mock".to_string(),
            active_model: "mock".to_string(),
            providers: vec![ProviderDescriptor {
                id: "mock".to_string(),
                capabilities: roder_api::inference::InferenceCapabilities::text_only(),
                models: vec![roder_api::inference::ModelDescriptor {
                    id: "mock".to_string(),
                    name: "Mock".to_string(),
                    context_window: None,
                }],
            }],
        };

        let options = provider_options_from_list(&list);
        assert_eq!(options.len(), 1);
        assert_eq!(options[0].provider_id, "mock");
        assert_eq!(options[0].model_id, "mock");
    }

    #[test]
    fn provider_menu_starts_with_models_submenu() {
        let items = main_provider_menu_items();
        assert!(matches!(items.first(), Some(ProviderMenuItem::Models)));
    }

    #[test]
    fn codex_auth_messages_reflect_status() {
        let signed_in = CodexAuthResult {
            signed_in: true,
            account_id: Some("acct".to_string()),
        };
        assert_eq!(
            codex_auth_message("auth/codex/status", &signed_in),
            "system: signed in with Codex account acct."
        );

        let signed_out = CodexAuthResult {
            signed_in: false,
            account_id: None,
        };
        assert_eq!(
            codex_auth_message("auth/codex/status", &signed_out),
            "system: signed out of Codex."
        );
    }
}
