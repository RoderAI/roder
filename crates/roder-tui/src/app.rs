mod commands;
mod dialog;
mod diff_ui;
mod help;
mod mouse_capture_runtime;
mod mouse_ui;
mod palette_ui;
mod selection_keyboard;

use std::collections::BTreeSet;
use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
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
use roder_api::inference::ProviderAuthType;
use roder_api::interactive::{
    HandlerOutcome, InteractiveEvent, InteractiveRegionHandler, MouseButton, RegionKind,
};
use roder_api::policy_mode::PolicyMode;
use roder_api::tui_status::{
    GitSnapshot, McpServerStatus, SessionSummary, StatusContext, StatusSegment,
};
use roder_app_server::LocalAppClient;
use roder_protocol::{
    AgentDescriptor, AgentsListResult, CodexAuthResult, CommandDescriptor, CommandsExpandParams,
    CommandsExpandResult, CommandsListResult, CreateSessionResult, InterruptTurnParams,
    JsonRpcRequest, JsonRpcResponse, PendingPlanExitDescriptor, ProviderDescriptor,
    ProviderSelectParams, ProviderSelectResult, ProvidersListResult, SessionExitPlanParams,
    SessionExitPlanResult, SessionGetResult, SessionLoadParams, SessionLoadResult,
    SessionResolveApprovalParams, SessionResolveApprovalResult, SessionSetModeParams,
    SessionSetModeResult, SessionsListResult, StartTurnParams, TasksListResult,
};
use tokio::process::Command;
use tui_textarea::TextArea;

use mouse_capture_runtime::apply_mouse_capture_event;

use crate::config::TuiAppConfig;
use crate::diff::{DiffViewerState, render::DiffTheme};
use crate::keymap::{Action, Keymap};
use crate::mouse::{
    DragSelectionOutcome, DragSelectionState, MouseCaptureController, MouseCaptureEvent,
    ScrollState, ScrollTarget, SelectedText, region_rect_from_ratatui,
};
use crate::palette::{PaletteEntry, render::PaletteTheme};
use crate::status_line::{
    StatusLineConfig, StatusLineTheme, built_in_status_segments, render_status_line,
};
use crate::transcript::{
    TranscriptAction, TranscriptContextMenu, TranscriptFoldState, action_for_region,
    context_menu_region, link_spans, transcript_regions,
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
    dialog_bg: Color,
    dialog_shadow: Color,
    dialog_key_bg: Color,
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
                dialog_bg: Color::Indexed(235),
                dialog_shadow: Color::Indexed(232),
                dialog_key_bg: Color::Indexed(238),
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
            dialog_bg: Color::Indexed(255),
            dialog_shadow: Color::Indexed(250),
            dialog_key_bg: Color::Indexed(252),
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

    fn dialog_surface(self) -> Style {
        Style::default().fg(self.text).bg(self.dialog_bg)
    }

    fn dialog_shadow(self) -> Style {
        Style::default().bg(self.dialog_shadow)
    }

    fn dialog_key(self) -> Style {
        Style::default()
            .fg(self.text_strong)
            .bg(self.dialog_key_bg)
            .add_modifier(Modifier::BOLD)
    }

    fn selected(self) -> Style {
        Style::default().fg(self.selection_fg).bg(self.selection_bg)
    }

    fn status_line(self) -> StatusLineTheme {
        StatusLineTheme {
            text: self.text,
            muted: self.muted,
            accent: self.accent,
            warning: self.tool,
            error: self.error,
            separator: self.subtle,
        }
    }

    fn palette(self) -> PaletteTheme {
        PaletteTheme {
            text: self.text,
            muted: self.muted,
            accent: self.accent,
            border: self.border,
            selection_fg: self.selection_fg,
            selection_bg: self.selection_bg,
            surface_bg: self.dialog_bg,
        }
    }

    fn diff(self) -> DiffTheme {
        DiffTheme {
            text: self.text,
            muted: self.muted,
            accent: self.accent,
            added: self.tool_running,
            removed: self.error,
            warning: self.tool,
            border: self.border,
            surface_bg: self.dialog_bg,
        }
    }
}

#[derive(Debug, Clone)]
struct ProviderOption {
    provider_id: String,
    model_id: String,
    label: String,
}

#[derive(Debug, Clone)]
struct ProviderChoice {
    provider_id: String,
    name: String,
    description: Option<String>,
    auth_type: ProviderAuthType,
    authenticated: bool,
    auth_detail: Option<String>,
    default_model: Option<String>,
    recommended: bool,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum ProviderPopupScreen {
    Main,
    Models,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum ConfirmDialog {
    Interrupt,
    Exit,
}

#[derive(Debug, Clone)]
enum ProviderMenuItem {
    Models,
    Provider(ProviderChoice),
    Model(ProviderOption),
    Back,
}

impl ProviderMenuItem {
    fn label(&self) -> String {
        match self {
            Self::Models => "Models".to_string(),
            Self::Provider(provider) => provider.label(),
            Self::Model(option) => option.label.clone(),
            Self::Back => "Back".to_string(),
        }
    }
}

impl ProviderChoice {
    fn label(&self) -> String {
        let mut label = self.name.clone();
        if self.recommended {
            label.push_str(" (Recommended)");
        } else if let Some(description) = &self.description {
            label.push_str(&format!(" ({description})"));
        }
        match self.auth_type {
            ProviderAuthType::OAuth if !self.authenticated => {
                label.push_str(" - sign in");
            }
            ProviderAuthType::OAuth if self.auth_detail.is_some() => {
                label.push_str(" - signed in");
            }
            ProviderAuthType::ApiKey => {
                label.push_str(" - API key");
            }
            _ => {}
        }
        label
    }
}

pub struct TuiApp {
    client: LocalAppClient,
    thread_id: String,
    active_turn_id: Option<String>,
    provider: String,
    model: String,
    composer: TextArea<'static>,
    messages: Vec<String>,
    events: Vec<String>,
    animation_frame: u64,
    show_event_log: bool,
    show_help: bool,
    show_provider_popup: bool,
    provider_popup_screen: ProviderPopupScreen,
    provider_choices: Vec<ProviderChoice>,
    model_options: Vec<ProviderOption>,
    provider_menu_items: Vec<ProviderMenuItem>,
    provider_menu_filter: String,
    provider_state: ListState,
    confirm_dialog: Option<ConfirmDialog>,
    command_catalog: Vec<CommandDescriptor>,
    slash_selected: usize,
    show_palette: bool,
    palette_entries: Vec<PaletteEntry>,
    palette_query: String,
    palette_source_filter: Option<String>,
    palette_state: ListState,
    enabled_palette_sources: BTreeSet<String>,
    diff_viewer: Option<DiffViewerState>,
    diff_enabled: bool,
    status_segments: Vec<StatusSegment>,
    interactive_region_handlers: Vec<Arc<dyn InteractiveRegionHandler>>,
    keymap: Keymap,
    status_config: StatusLineConfig,
    status_git: Option<GitSnapshot>,
    policy_mode: PolicyMode,
    pending_plan_exit: Option<PendingPlanExitDescriptor>,
    mouse_feedback: mouse_ui::MouseFeedbackState,
    drag_selection: DragSelectionState,
    transcript_scroll: ScrollState,
    mouse_capture: MouseCaptureController,
    selection_keyboard: selection_keyboard::SelectionKeyboardState,
    transcript_fold: TranscriptFoldState,
    transcript_context_menu: Option<TranscriptContextMenu>,
    terminal_area: roder_api::interactive::RegionRect,
    theme: Theme,
}

impl TuiApp {
    pub async fn new(client: LocalAppClient, model: String) -> anyhow::Result<Self> {
        Self::new_with_config(client, model, TuiAppConfig::default()).await
    }

    pub async fn new_with_config(
        client: LocalAppClient,
        model: String,
        config: TuiAppConfig,
    ) -> anyhow::Result<Self> {
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
        let theme = Theme::for_terminal();
        let policy_state = session_get(&client).await.ok();
        let command_catalog = commands_list(&client).await.unwrap_or_default();
        let status_git = current_git_snapshot();

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
            composer: composer_textarea(theme),
            messages: Vec::new(),
            events: Vec::new(),
            animation_frame: 0,
            show_event_log: false,
            show_help: false,
            show_provider_popup: false,
            provider_popup_screen: ProviderPopupScreen::Main,
            provider_choices: Vec::new(),
            model_options: Vec::new(),
            provider_menu_items: Vec::new(),
            provider_menu_filter: String::new(),
            provider_state,
            confirm_dialog: None,
            command_catalog,
            slash_selected: 0,
            show_palette: false,
            palette_entries: Vec::new(),
            palette_query: String::new(),
            palette_source_filter: None,
            palette_state: ListState::default(),
            enabled_palette_sources: config.enabled_palette_source_ids(),
            diff_viewer: None,
            diff_enabled: config.diff_enabled,
            status_segments: if config.status_segments.is_empty() {
                built_in_status_segments()
            } else {
                config.status_segments
            },
            status_config: StatusLineConfig {
                disabled_segments: config.disabled_status_segments,
            },
            interactive_region_handlers: config.interactive_region_handlers,
            keymap: config.keymap,
            status_git,
            policy_mode: policy_state
                .as_ref()
                .map(|state| state.mode)
                .unwrap_or_default(),
            pending_plan_exit: policy_state.and_then(|state| state.pending_plan_exit),
            mouse_feedback: mouse_ui::MouseFeedbackState::default(),
            drag_selection: DragSelectionState::default(),
            transcript_scroll: ScrollState::default(),
            mouse_capture: MouseCaptureController::default(),
            selection_keyboard: selection_keyboard::SelectionKeyboardState::default(),
            transcript_fold: TranscriptFoldState::default(),
            transcript_context_menu: None,
            terminal_area: roder_api::interactive::RegionRect {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
            },
            theme,
        })
    }

    pub async fn run(&mut self) -> anyhow::Result<()> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let mut rx = self.client.subscribe_events();

        loop {
            self.animation_frame = self.animation_frame.wrapping_add(1);
            if let Some(event) = self.mouse_capture.tick(self.animation_frame) {
                apply_mouse_capture_event(terminal.backend_mut(), event)?;
                self.push_event(mouse_capture_event_label(event).to_string());
            }
            terminal.draw(|f| self.render(f))?;

            if event::poll(Duration::from_millis(50))? {
                match event::read()? {
                    Event::Key(key) => {
                        if self.confirm_dialog.is_some() {
                            if self.handle_confirm_key(key).await {
                                break;
                            }
                        } else if self.show_help {
                            self.handle_help_key(key);
                        } else if help::is_help_key(key) {
                            self.show_help = true;
                            self.push_event("help shown".to_string());
                        } else if self.diff_viewer.is_some() {
                            self.handle_diff_key(key).await;
                        } else if self.show_palette {
                            self.handle_palette_key(key).await;
                        } else if palette_ui::is_palette_open_key(key) {
                            self.open_palette().await;
                        } else if key.modifiers.contains(KeyModifiers::CONTROL)
                            && key.code == KeyCode::Char('p')
                        {
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
                        } else if self.handle_selection_keyboard_action(key) {
                        } else if key.modifiers.contains(KeyModifiers::CONTROL)
                            && key.code == KeyCode::Char('c')
                        {
                            self.confirm_dialog = Some(ConfirmDialog::Exit);
                        } else if key.code == KeyCode::BackTab {
                            self.cycle_policy_mode().await;
                        } else if self.pending_plan_exit.is_some()
                            && matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y'))
                        {
                            self.resolve_pending_plan_exit(true).await;
                        } else if self.pending_plan_exit.is_some()
                            && matches!(key.code, KeyCode::Char('n') | KeyCode::Char('N'))
                        {
                            self.resolve_pending_plan_exit(false).await;
                        } else if self.show_provider_popup {
                            match key.code {
                                KeyCode::Esc => self.close_or_back_provider_popup(),
                                KeyCode::Up => self.select_previous_provider_menu_item(),
                                KeyCode::Down => self.select_next_provider_menu_item(),
                                KeyCode::Enter => self.select_current_provider_menu_item().await,
                                KeyCode::Backspace => {
                                    self.provider_menu_filter.pop();
                                    self.clamp_provider_menu_selection();
                                }
                                KeyCode::Char(c)
                                    if !key.modifiers.contains(KeyModifiers::CONTROL) =>
                                {
                                    self.provider_menu_filter.push(c);
                                    self.clamp_provider_menu_selection();
                                }
                                _ => {}
                            }
                        } else if self.command_menu_open() {
                            match key.code {
                                KeyCode::Up => self.move_slash_selection(-1),
                                KeyCode::Down => self.move_slash_selection(1),
                                KeyCode::Tab => self.accept_slash_completion(),
                                KeyCode::Enter => {
                                    if key.modifiers.contains(KeyModifiers::SHIFT) {
                                        self.composer.insert_newline();
                                        continue;
                                    }
                                    let text = composer_text(&self.composer).trim().to_string();
                                    self.composer = composer_textarea(self.theme);
                                    if text.is_empty() {
                                        continue;
                                    }
                                    if self.try_run_slash_command(&text).await {
                                        continue;
                                    }
                                    self.submit_user_text(text).await;
                                }
                                _ => {
                                    self.composer.input(key);
                                    self.clamp_slash_selection();
                                }
                            }
                        } else {
                            match key.code {
                                KeyCode::Enter => {
                                    if key.modifiers.contains(KeyModifiers::SHIFT) {
                                        self.composer.insert_newline();
                                        continue;
                                    }
                                    let text = composer_text(&self.composer).trim().to_string();
                                    self.composer = composer_textarea(self.theme);
                                    if text.is_empty() {
                                        continue;
                                    }
                                    if text.starts_with('/')
                                        && self.try_run_slash_command(&text).await
                                    {
                                        continue;
                                    }
                                    if let Some(command) = shell_command_from_input(&text) {
                                        self.run_shell_command(command).await;
                                        continue;
                                    }
                                    self.submit_user_text(text).await;
                                }
                                KeyCode::Esc => {
                                    self.confirm_dialog = if self.active_turn_id.is_some() {
                                        Some(ConfirmDialog::Interrupt)
                                    } else {
                                        Some(ConfirmDialog::Exit)
                                    };
                                }
                                _ => {
                                    self.composer.input(key);
                                    self.clamp_slash_selection();
                                }
                            }
                        }
                    }
                    Event::Mouse(mouse) => {
                        let at = (mouse.column, mouse.row);
                        let events = self
                            .mouse_feedback
                            .handle_mouse_event(mouse, Instant::now());
                        for event in self.handle_interactive_events(events, at).await {
                            apply_mouse_capture_event(terminal.backend_mut(), event)?;
                            self.push_event(mouse_capture_event_label(event).to_string());
                        }
                    }
                    _ => {}
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
                    RoderEvent::InferenceEventReceived(ev) => match ev.event {
                        roder_api::inference::InferenceEvent::MessageDelta(delta) => {
                            if let Some(last) = self.messages.last_mut()
                                && last.starts_with("assistant: ")
                            {
                                last.push_str(&delta.text);
                                continue;
                            }
                            self.messages.push(format!("assistant: {}", delta.text));
                        }
                        roder_api::inference::InferenceEvent::ToolCallStarted(tool) => {
                            self.messages
                                .push(format!("tool call: {} {}", tool.id, tool.name));
                        }
                        roder_api::inference::InferenceEvent::ToolCallCompleted(tool) => {
                            self.messages
                                .push(format!("tool call: {} {} completed", tool.id, tool.name));
                        }
                        _ => {}
                    },
                    RoderEvent::TurnFailed(ev) => {
                        self.messages.push(format!("error: {}", ev.error))
                    }
                    RoderEvent::PolicyModeChanged(ev) => {
                        self.policy_mode = ev.new_mode;
                        self.push_event(format!(
                            "policy mode changed: {}",
                            policy_mode_label(ev.new_mode)
                        ));
                    }
                    RoderEvent::PolicyExitPlanRequested(_) => {
                        self.refresh_session_state().await;
                    }
                    RoderEvent::PolicyExitPlanResolved(_) => {
                        self.refresh_session_state().await;
                    }
                    RoderEvent::FileChangePreviewReady(ev) if self.diff_enabled => {
                        self.open_diff_preview(ev);
                    }
                    RoderEvent::ApprovalRequested(ev) if !self.diff_enabled => {
                        self.resolve_diff_approval(ev.approval_id, false).await;
                    }
                    _ => {}
                }
            }
        }

        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            DisableMouseCapture,
            LeaveAlternateScreen
        )?;
        terminal.show_cursor()?;

        Ok(())
    }

    async fn handle_confirm_key(&mut self, key: crossterm::event::KeyEvent) -> bool {
        let Some(dialog) = self.confirm_dialog else {
            return false;
        };
        match key.code {
            KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                self.confirm_dialog = None;
                match dialog {
                    ConfirmDialog::Interrupt => self.interrupt_active_turn().await,
                    ConfirmDialog::Exit => return true,
                }
            }
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                self.confirm_dialog = None;
            }
            _ => {}
        }
        false
    }

    fn handle_help_key(&mut self, key: crossterm::event::KeyEvent) {
        if matches!(key.code, KeyCode::Esc | KeyCode::Char('?')) {
            self.show_help = false;
            self.push_event("help hidden".to_string());
        }
    }

    async fn interrupt_active_turn(&mut self) {
        let Some(turn_id) = self.active_turn_id.clone() else {
            self.messages
                .push("system: no running turn to interrupt.".to_string());
            return;
        };
        let params = InterruptTurnParams {
            thread_id: self.thread_id.clone(),
            turn_id,
        };
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("interrupt")),
                method: "turns/interrupt".to_string(),
                params: Some(serde_json::to_value(params).unwrap()),
            })
            .await;
        if let Some(err) = res.error {
            self.record_error(format!("interrupt failed: {}", err.message));
        } else {
            self.push_event("interrupt requested".to_string());
        }
    }

    async fn refresh_session_state(&mut self) {
        match session_get(&self.client).await {
            Ok(state) => {
                self.policy_mode = state.mode;
                self.pending_plan_exit = state.pending_plan_exit;
            }
            Err(err) => self.record_error(format!("session/get failed: {err}")),
        }
    }

    async fn cycle_policy_mode(&mut self) {
        let next = next_policy_mode(self.policy_mode);
        self.set_policy_mode(next, "tui mode switcher").await;
    }

    async fn set_policy_mode(&mut self, mode: PolicyMode, reason: &str) {
        let params = SessionSetModeParams {
            mode,
            reason: Some(reason.to_string()),
        };
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("session/set_mode")),
                method: "session/set_mode".to_string(),
                params: Some(serde_json::to_value(params).unwrap()),
            })
            .await;
        match decode_response::<SessionSetModeResult>(res) {
            Ok(result) => {
                self.policy_mode = result.mode;
                self.messages.push(format!(
                    "system: policy mode set to {}.",
                    policy_mode_label(result.mode)
                ));
                self.push_event(format!(
                    "policy mode selected: {}",
                    policy_mode_label(result.mode)
                ));
            }
            Err(err) => self.record_error(format!("session/set_mode failed: {err}")),
        }
    }

    async fn load_session(&mut self, thread_id: String) {
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("sessions/load")),
                method: "sessions/load".to_string(),
                params: Some(
                    serde_json::to_value(SessionLoadParams {
                        thread_id: thread_id.clone(),
                    })
                    .unwrap(),
                ),
            })
            .await;
        match decode_response::<SessionLoadResult>(res) {
            Ok(result) => {
                self.thread_id = thread_id;
                self.active_turn_id = None;
                self.messages.clear();
                if let Some(metadata) = result.snapshot.and_then(|snapshot| snapshot.metadata) {
                    if let Some(provider) = metadata.provider {
                        self.provider = provider;
                    }
                    if let Some(model) = metadata.model {
                        self.model = model;
                    }
                }
                self.messages.push(format!(
                    "system: loaded session {}.",
                    short_id(&self.thread_id)
                ));
                self.push_event(format!("session loaded: {}", short_id(&self.thread_id)));
            }
            Err(err) => self.record_error(format!("sessions/load failed: {err}")),
        }
    }

    async fn resolve_pending_plan_exit(&mut self, approved: bool) {
        let Some(pending) = self.pending_plan_exit.clone() else {
            return;
        };
        let params = SessionExitPlanParams {
            request_id: pending.request_id.clone(),
            approved,
        };
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("session/exit_plan")),
                method: "session/exit_plan".to_string(),
                params: Some(serde_json::to_value(params).unwrap()),
            })
            .await;
        match decode_response::<SessionExitPlanResult>(res) {
            Ok(result) => {
                self.policy_mode = result.mode;
                self.pending_plan_exit = None;
                self.messages.push(format!(
                    "system: {} plan exit request {}.",
                    if approved { "approved" } else { "rejected" },
                    short_id(&pending.request_id)
                ));
            }
            Err(err) => self.record_error(format!("session/exit_plan failed: {err}")),
        }
    }

    async fn try_run_slash_command(&mut self, text: &str) -> bool {
        if text.trim() == "/tasks" {
            self.show_tasks().await;
            return true;
        }
        let Some((name, arguments)) = commands::command_invocation(text, &self.command_catalog)
        else {
            return false;
        };

        match commands_expand(&self.client, &name, &arguments).await {
            Ok(expanded) => {
                self.messages
                    .push(format!("executed slash command: /{name}"));
                self.messages.push(format!(
                    "command preview: {}",
                    truncate(&expanded.message.replace('\n', " "), 120)
                ));
                self.submit_user_message(
                    expanded.message,
                    expanded.model.or_else(|| Some(self.model.clone())),
                )
                .await;
            }
            Err(err) => self.record_error(format!("slash command failed: {err}")),
        }
        true
    }

    async fn show_tasks(&mut self) {
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("tasks/list")),
                method: "tasks/list".to_string(),
                params: None,
            })
            .await;
        match decode_response::<TasksListResult>(res) {
            Ok(result) if result.tasks.is_empty() => {
                self.messages
                    .push("system: no background tasks.".to_string());
            }
            Ok(result) => {
                self.messages.push("system: background tasks".to_string());
                for task in result.tasks.iter().rev().take(6).rev() {
                    self.messages.push(format!(
                        "task {} {} {:?}",
                        short_id(&task.task_id),
                        task.spec.kind,
                        task.state
                    ));
                }
            }
            Err(err) => self.record_error(format!("tasks/list failed: {err}")),
        }
    }

    async fn submit_user_text(&mut self, text: String) {
        self.messages.push(format!("user: {text}"));
        self.submit_user_message(text, Some(self.model.clone()))
            .await;
    }

    async fn submit_user_message(&self, message: String, model_override: Option<String>) {
        let params = StartTurnParams {
            thread_id: self.thread_id.clone(),
            message,
            provider_override: None,
            model_override,
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

    fn command_menu_open(&self) -> bool {
        !self.matching_slash_commands().is_empty()
    }

    fn matching_slash_commands(&self) -> Vec<&CommandDescriptor> {
        commands::matching_commands(&self.command_catalog, &composer_text(&self.composer))
    }

    fn move_slash_selection(&mut self, delta: isize) {
        let count = self.matching_slash_commands().len();
        if count == 0 {
            self.slash_selected = 0;
            return;
        }
        self.slash_selected =
            (self.slash_selected as isize + delta).rem_euclid(count as isize) as usize;
    }

    fn clamp_slash_selection(&mut self) {
        let count = self.matching_slash_commands().len();
        if count == 0 {
            self.slash_selected = 0;
        } else {
            self.slash_selected = self.slash_selected.min(count - 1);
        }
    }

    fn accept_slash_completion(&mut self) {
        if let Some(completed) = commands::accepted_completion(
            &composer_text(&self.composer),
            &self.command_catalog,
            self.slash_selected,
        ) {
            self.composer = composer_textarea(self.theme);
            self.composer.insert_str(completed);
            self.slash_selected = 0;
        }
    }

    async fn run_shell_command(&mut self, command: String) {
        self.messages.push(format!("shell: !{command}"));
        self.push_event(format!("shell command started: {command}"));
        match run_shell_command(command.clone()).await {
            Ok(output) => {
                self.messages.push(format!("shell output: {output}"));
                self.push_event(format!("shell command finished: {command}"));
            }
            Err(err) => {
                self.record_error(format!("shell command failed: {err}"));
            }
        }
    }

    fn render(&mut self, f: &mut Frame<'_>) {
        let area = f.area();
        self.terminal_area = region_rect_from_ratatui(area);
        let event_height = event_log_height(self.show_event_log, self.events.len());
        let loader_height = if self.active_turn_id.is_some() { 1 } else { 0 };
        let command_height = command_menu_height(self.command_menu_open());
        let mut constraints = Vec::new();
        if loader_height > 0 {
            constraints.push(Constraint::Length(loader_height));
        }
        constraints.extend([Constraint::Length(1), Constraint::Min(5)]);
        if event_height > 0 {
            constraints.push(Constraint::Length(event_height));
        }
        if command_height > 0 {
            constraints.push(Constraint::Length(command_height));
        }
        constraints.extend([Constraint::Length(3), Constraint::Length(1)]);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(area);

        let header_index = if loader_height > 0 {
            f.render_widget(self.animated_top_bar(area.width), chunks[0]);
            1
        } else {
            0
        };
        let transcript_index = header_index + 1;
        let transcript_area = chunks[transcript_index];
        self.transcript_scroll.set_target(if self.show_palette {
            ScrollTarget::Palette
        } else if self.diff_viewer.is_some() {
            ScrollTarget::Diff
        } else {
            ScrollTarget::Transcript
        });
        self.transcript_scroll.set_bounds(
            transcript_content_rows(self.visible_transcript_messages().len()),
            usize::from(transcript_area.height),
        );
        f.render_widget(self.header(area.width), chunks[header_index]);
        f.render_widget(self.transcript(), chunks[transcript_index]);

        let mut composer_index = if event_height > 0 {
            f.render_widget(self.event_log(), chunks[transcript_index + 1]);
            transcript_index + 2
        } else {
            transcript_index + 1
        };
        if command_height > 0 {
            f.render_widget(self.command_menu(), chunks[composer_index]);
            composer_index += 1;
        }
        let composer_area = chunks[composer_index];
        let footer_area = chunks[composer_index + 1];
        let mut interactive_regions = transcript_regions(
            &self.scrolled_transcript_messages(),
            &self.thread_id,
            self.active_turn_id.as_deref().unwrap_or("tui"),
            region_rect_from_ratatui(transcript_area),
            &self.transcript_fold,
        );
        if self.show_palette {
            interactive_regions.extend(self.palette_item_regions(area));
        }
        self.mouse_feedback
            .set_frame_regions(composer_area, footer_area, interactive_regions);
        let composer_border_style = self
            .mouse_feedback
            .style_for_region(mouse_ui::COMPOSER_REGION_ID, self.theme.border());
        self.composer
            .set_block(composer_block(self.theme, composer_border_style));
        f.render_widget(&self.composer, chunks[composer_index]);
        f.render_widget(self.footer(area.width), chunks[composer_index + 1]);

        if self.show_provider_popup {
            self.render_provider_popup(f, area);
        }
        if self.show_palette {
            self.render_palette_popup(f, area);
        }
        if self.diff_viewer.is_some() {
            self.render_diff_viewer(f, area);
        }
        if self.show_help {
            help::render_keymap_help(f, area, &self.keymap, self.theme);
        }
        if let Some(menu) = &self.transcript_context_menu {
            self.render_transcript_context_menu(f, menu);
        }
        if let Some(dialog) = self.confirm_dialog {
            self.render_confirm_dialog(f, area, dialog);
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
        let visible_messages = self.scrolled_transcript_messages();
        let text = if visible_messages.is_empty() {
            Text::from(vec![
                Line::raw(""),
                Line::from(Span::styled(
                    "No transcript yet. Ask Roder to inspect, edit, or run something.",
                    self.theme.muted().add_modifier(Modifier::ITALIC),
                )),
            ])
        } else {
            Text::from(
                visible_messages
                    .iter()
                    .flat_map(|message| [message_line(message, self.theme), Line::raw("")])
                    .collect::<Vec<_>>(),
            )
        };

        Paragraph::new(text).style(self.theme.text())
    }

    fn visible_transcript_messages(&self) -> Vec<String> {
        self.messages
            .iter()
            .enumerate()
            .map(|(idx, message)| self.transcript_fold.visible_message(idx, message))
            .collect()
    }

    fn scrolled_transcript_messages(&self) -> Vec<String> {
        let skip_messages = self.transcript_scroll.offset() / 2;
        self.visible_transcript_messages()
            .into_iter()
            .skip(skip_messages)
            .collect()
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

    fn command_menu(&self) -> Paragraph<'static> {
        let matches = self.matching_slash_commands();
        let selected = self.slash_selected.min(matches.len().saturating_sub(1));
        let lines = matches
            .into_iter()
            .take(4)
            .enumerate()
            .map(|(index, command)| {
                let style = if index == selected {
                    self.theme.selected()
                } else {
                    self.theme.text()
                };
                let mut spans = vec![
                    Span::styled(
                        if index == selected { "› " } else { "  " },
                        self.theme.subtle(),
                    ),
                    Span::styled(format!("/{}", command.name), style),
                ];
                if let Some(hint) = &command.argument_hint {
                    spans.push(Span::styled(format!(" {hint}"), self.theme.muted()));
                }
                if let Some(description) = &command.description {
                    spans.push(Span::styled(
                        format!("  {}", truncate(description, 44)),
                        self.theme.muted(),
                    ));
                }
                if let Some(warning) = commands::command_warning(command) {
                    spans.push(Span::styled(format!("  {warning}"), self.theme.tool()));
                }
                Line::from(spans)
            })
            .collect::<Vec<_>>();

        Paragraph::new(Text::from(lines)).block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(self.theme.border())
                .title(Span::styled(" commands  tab complete ", self.theme.muted())),
        )
    }

    fn footer(&self, width: u16) -> Paragraph<'static> {
        let session = SessionSummary {
            thread_id: self.thread_id.clone(),
            title: None,
        };
        let mcp: &[McpServerStatus] = &[];
        let ctx = StatusContext {
            session: &session,
            policy_mode: self.policy_mode,
            model: Some(&self.model),
            usage: None,
            git: self.status_git.as_ref(),
            mcp,
        };
        let mut segments = self.status_segments.clone();
        if let Some(segment) = self.mouse_feedback.status_segment() {
            segments.push(segment);
        }
        render_status_line(
            &segments,
            &ctx,
            width,
            &self.status_config,
            self.theme.status_line(),
        )
    }

    fn render_provider_popup(&mut self, f: &mut Frame<'_>, area: Rect) {
        let menu_area = centered_rect(area, area.width.min(72), area.height.min(16));
        let visible_items = self.filtered_provider_menu_items();
        let items: Vec<ListItem> = if visible_items.is_empty() {
            vec![ListItem::new(Line::from(Span::styled(
                "No matches",
                self.theme.muted(),
            )))]
        } else {
            visible_items
                .iter()
                .map(|item| {
                    let marker = match item {
                        ProviderMenuItem::Provider(provider) if provider.authenticated => "✓ ",
                        _ => "• ",
                    };
                    ListItem::new(Line::from(vec![
                        Span::styled(marker, self.theme.subtle()),
                        Span::styled(item.label(), self.theme.text()),
                    ]))
                })
                .collect()
        };
        let title = match self.provider_popup_screen {
            ProviderPopupScreen::Main => " Connect a provider (Enter select, Esc close) ",
            ProviderPopupScreen::Models => " Models (Enter select, Esc back) ",
        };
        let title = if self.provider_menu_filter.is_empty() {
            title.to_string()
        } else {
            format!("{} /{} ", title.trim_end(), self.provider_menu_filter)
        };
        let menu = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .style(self.theme.dialog_surface())
                    .border_style(self.theme.dialog())
                    .title(Span::styled(title, self.theme.accent())),
            )
            .style(self.theme.dialog_surface())
            .highlight_style(self.theme.selected())
            .highlight_symbol("› ");
        f.render_widget(Clear, menu_area);
        f.render_stateful_widget(menu, menu_area, &mut self.provider_state);
    }

    fn render_confirm_dialog(&self, f: &mut Frame<'_>, area: Rect, dialog: ConfirmDialog) {
        dialog::render_confirm_dialog(f, area, dialog, self.theme);
    }

    fn render_transcript_context_menu(&self, f: &mut Frame<'_>, menu: &TranscriptContextMenu) {
        let area = Rect::new(menu.area.x, menu.area.y, menu.area.width, menu.area.height);
        let text = Text::from(vec![
            Line::from(Span::styled("copy", self.theme.text())),
            Line::from(Span::styled("fold", self.theme.text())),
            Line::from(Span::styled("jump", self.theme.text())),
        ]);
        let widget = Paragraph::new(text)
            .style(self.theme.dialog_surface())
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(self.theme.dialog())
                    .title(Span::styled(
                        format!(" msg {} ", menu.message_idx),
                        self.theme.accent(),
                    )),
            );
        f.render_widget(Clear, area);
        f.render_widget(widget, area);
    }

    async fn handle_interactive_events(
        &mut self,
        events: Vec<InteractiveEvent>,
        at: (u16, u16),
    ) -> Vec<MouseCaptureEvent> {
        let mut capture_events = Vec::new();
        for event in events {
            self.handle_drag_selection_event(&event);
            match &event {
                InteractiveEvent::Scroll {
                    delta_lines,
                    modifiers,
                    ..
                } => {
                    if let Some(event) = self.handle_scroll_event(*delta_lines, *modifiers) {
                        capture_events.push(event);
                    }
                }
                InteractiveEvent::Click {
                    region,
                    button: MouseButton::Left,
                    ..
                }
                | InteractiveEvent::DoubleClick { region, .. } => {
                    self.transcript_context_menu = None;
                    self.handle_region_with_extensions(&event, region, None)
                        .await;
                }
                InteractiveEvent::RightClick { region, .. } => {
                    self.handle_region_with_extensions(&event, region, Some(at))
                        .await;
                }
                _ => {}
            }
        }
        capture_events
    }

    async fn handle_region_with_extensions(
        &mut self,
        event: &InteractiveEvent,
        region: &str,
        right_click_at: Option<(u16, u16)>,
    ) {
        if !self.dispatch_region_handlers(event, region).await {
            self.handle_region_action(region, right_click_at).await;
        }
    }

    async fn dispatch_region_handlers(
        &mut self,
        event: &InteractiveEvent,
        region_id: &str,
    ) -> bool {
        let Some(region) = self.mouse_feedback.region(region_id).cloned() else {
            return false;
        };
        let kind = region_kind_name(&region.kind);
        let handlers = self.interactive_region_handlers.clone();
        for handler in handlers {
            if !handler.kinds().iter().any(|candidate| candidate == kind) {
                continue;
            }
            match handler.handle(event.clone(), &region).await {
                Ok(HandlerOutcome::Consumed) => return true,
                Ok(HandlerOutcome::InvalidateRender) => {
                    self.push_event(format!("interactive region invalidated: {region_id}"));
                    return true;
                }
                Ok(HandlerOutcome::Passthrough) => {}
                Err(err) => self.record_error(format!("interactive handler failed: {err}")),
            }
        }
        false
    }

    fn handle_scroll_event(
        &mut self,
        delta_lines: i16,
        modifiers: roder_api::interactive::KeyModifiers,
    ) -> Option<MouseCaptureEvent> {
        let outcome = self.transcript_scroll.scroll(delta_lines, modifiers);
        if outcome.at_boundary {
            let event = self
                .mouse_capture
                .release_for_boundary_scroll(self.animation_frame);
            self.push_event(format!("scroll boundary: {:?}", outcome.target));
            return event;
        }
        None
    }

    fn handle_drag_selection_event(&mut self, event: &InteractiveEvent) {
        let Some(region_id) = interactive_region_id(event) else {
            return;
        };
        let region = self.mouse_feedback.region(region_id).cloned();
        let transcript_lines = self.visible_transcript_messages();
        let composer_value = composer_text(&self.composer);
        let Some(outcome) = self.drag_selection.apply_event(
            event,
            region.as_ref(),
            &transcript_lines,
            &composer_value,
        ) else {
            return;
        };
        match outcome {
            DragSelectionOutcome::Started(_) | DragSelectionOutcome::Updated => {}
            DragSelectionOutcome::Finalized(SelectedText::Transcript(text)) => {
                self.selection_keyboard
                    .remember(SelectedText::Transcript(text.clone()));
                self.push_event(format!(
                    "transcript selection finalized: {} chars",
                    text.chars().count()
                ));
            }
            DragSelectionOutcome::Finalized(SelectedText::Composer(text)) => {
                self.selection_keyboard
                    .remember(SelectedText::Composer(text.clone()));
                self.push_event(format!(
                    "composer selection finalized: {} chars",
                    text.chars().count()
                ));
            }
            DragSelectionOutcome::ClearedTooShort => {
                self.push_event("selection cleared: too short".to_string());
            }
        }
    }

    fn handle_selection_keyboard_action(&mut self, key: crossterm::event::KeyEvent) -> bool {
        if self.keymap.matches_key_event(Action::CopySelection, &key) {
            match self.selection_keyboard.copy_last_selection() {
                Ok(Some(chars)) => self.push_event(format!("selection copied: {chars} chars")),
                Ok(None) => self.push_event("selection copy skipped: no selection".to_string()),
                Err(err) => self.record_error(format!("selection copy failed: {err}")),
            }
            return true;
        }

        if self.keymap.matches_key_event(Action::PasteToComposer, &key) {
            let Some(text) = self.selection_keyboard.paste_text() else {
                self.push_event("selection paste skipped: empty clipboard".to_string());
                return true;
            };
            let chars = text.chars().count();
            self.composer.insert_str(text);
            self.clamp_slash_selection();
            self.push_event(format!("selection pasted: {chars} chars"));
            return true;
        }

        false
    }

    async fn handle_region_action(&mut self, region_id: &str, right_click_at: Option<(u16, u16)>) {
        let Some(region) = self.mouse_feedback.region(region_id).cloned() else {
            return;
        };
        if let RegionKind::PaletteItem { source_id, item_id } = region.kind {
            self.execute_palette_item(&source_id, &item_id).await;
            return;
        }
        let Some(action) = action_for_region(&region, right_click_at) else {
            return;
        };
        match action {
            TranscriptAction::ToggleToolCall { call_id } => {
                let expanded = self.transcript_fold.toggle_tool_call(call_id.clone());
                self.push_event(format!(
                    "tool call {}: {call_id}",
                    if expanded { "expanded" } else { "collapsed" }
                ));
            }
            TranscriptAction::ToggleMessage { message_idx } => {
                let expanded = self.transcript_fold.toggle_message(message_idx);
                self.push_event(format!(
                    "message {}: {message_idx}",
                    if expanded { "expanded" } else { "folded" }
                ));
            }
            TranscriptAction::OpenUrl { url } => {
                self.messages.push(format!("system: selected URL {url}."));
                self.push_event(format!("url selected: {url}"));
            }
            TranscriptAction::OpenFile { path, line } => {
                let suffix = line.map(|line| format!(":{line}")).unwrap_or_default();
                self.messages
                    .push(format!("system: selected file {path}{suffix}."));
                self.push_event(format!("file selected: {path}{suffix}"));
            }
            TranscriptAction::OpenContextMenu { message_idx, at } => {
                self.transcript_context_menu =
                    Some(context_menu_region(message_idx, at, self.terminal_area));
            }
        }
    }

    async fn open_provider_popup(&mut self) {
        match self.providers_list().await {
            Ok(list) => {
                self.provider = list.active_provider.clone();
                self.model = list.active_model.clone();
                self.provider_choices = provider_choices_from_list(&list);
                self.model_options = provider_options_from_list(&list);
                self.provider_menu_items = main_provider_menu_items(&self.provider_choices);
                self.provider_popup_screen = ProviderPopupScreen::Main;
                self.provider_menu_filter.clear();
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
        if !self.provider_menu_filter.is_empty() {
            self.provider_menu_filter.clear();
            self.clamp_provider_menu_selection();
            return;
        }
        if self.provider_popup_screen == ProviderPopupScreen::Models {
            self.provider_popup_screen = ProviderPopupScreen::Main;
            self.provider_menu_items = main_provider_menu_items(&self.provider_choices);
            self.provider_state.select(Some(0));
        } else {
            self.show_provider_popup = false;
        }
    }

    fn select_previous_provider_menu_item(&mut self) {
        let visible_len = self.filtered_provider_menu_items().len();
        if visible_len == 0 {
            return;
        }
        let last = visible_len - 1;
        let i = match self.provider_state.selected() {
            Some(0) | None => last,
            Some(i) => i - 1,
        };
        self.provider_state.select(Some(i));
    }

    fn select_next_provider_menu_item(&mut self) {
        let visible_len = self.filtered_provider_menu_items().len();
        if visible_len == 0 {
            return;
        }
        let last = visible_len - 1;
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
        let Some(item) = self.filtered_provider_menu_items().get(selected).cloned() else {
            self.show_provider_popup = false;
            return;
        };

        match item {
            ProviderMenuItem::Models => {
                self.open_models_submenu();
            }
            ProviderMenuItem::Provider(provider) => {
                self.select_provider(provider).await;
            }
            ProviderMenuItem::Model(option) => {
                self.select_provider_model(option).await;
            }
            ProviderMenuItem::Back => {
                self.provider_popup_screen = ProviderPopupScreen::Main;
                self.provider_menu_filter.clear();
                self.provider_menu_items = main_provider_menu_items(&self.provider_choices);
                self.provider_state.select(Some(0));
            }
        }
    }

    fn open_models_submenu(&mut self) {
        self.provider_popup_screen = ProviderPopupScreen::Models;
        self.provider_menu_filter.clear();
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

    fn filtered_provider_menu_items(&self) -> Vec<ProviderMenuItem> {
        filter_provider_menu_items(&self.provider_menu_items, &self.provider_menu_filter)
    }

    fn clamp_provider_menu_selection(&mut self) {
        let len = self.filtered_provider_menu_items().len();
        if len == 0 {
            self.provider_state.select(None);
            return;
        }
        let selected = self.provider_state.selected().unwrap_or(0).min(len - 1);
        self.provider_state.select(Some(selected));
    }

    async fn select_provider_model(&mut self, option: ProviderOption) {
        let params = ProviderSelectParams {
            provider: option.provider_id,
            model: Some(option.model_id),
        };
        self.select_provider_model_params(params).await;
    }

    async fn select_provider(&mut self, provider: ProviderChoice) {
        if provider.auth_type == ProviderAuthType::OAuth && !provider.authenticated {
            if provider.provider_id != "codex" {
                self.record_error(format!(
                    "provider {} requires OAuth but has no login flow",
                    provider.provider_id
                ));
                self.show_provider_popup = false;
                return;
            }
            if !self.run_codex_auth("auth/codex/login").await {
                return;
            }
        }
        let params = ProviderSelectParams {
            provider: provider.provider_id,
            model: provider.default_model,
        };
        self.select_provider_model_params(params).await;
    }

    async fn select_provider_model_params(&mut self, params: ProviderSelectParams) {
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

    async fn run_codex_auth(&mut self, method: &str) -> bool {
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
                true
            }
            Err(err) => {
                self.record_error(format!("codex auth failed: {err}"));
                self.show_provider_popup = false;
                false
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
        spans.push(Span::styled("─".repeat(offset), Style::default().fg(track)));
    }
    spans.push(Span::styled(
        "─".repeat(highlight_width),
        Style::default().fg(fill),
    ));
    let tail = width.saturating_sub(offset + highlight_width);
    if tail > 0 {
        spans.push(Span::styled("─".repeat(tail), Style::default().fg(track)));
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

fn transcript_content_rows(message_count: usize) -> usize {
    message_count.saturating_mul(2)
}

fn command_menu_height(open: bool) -> u16 {
    if open { 5 } else { 0 }
}

fn composer_textarea(theme: Theme) -> TextArea<'static> {
    let mut composer = TextArea::default();
    composer.set_block(composer_block(theme, theme.border()));
    composer.set_style(theme.text());
    composer.set_cursor_line_style(theme.text());
    composer.set_cursor_style(
        Style::default()
            .fg(theme.selection_fg)
            .bg(theme.selection_bg),
    );
    composer.set_placeholder_text("Ask Roder to work on this repo");
    composer.set_placeholder_style(theme.muted().add_modifier(Modifier::ITALIC));
    composer
}

fn composer_block(theme: Theme, border_style: Style) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        .title(Span::styled(" composer ", theme.muted()))
}

fn composer_text(composer: &TextArea<'_>) -> String {
    composer.lines().join("\n")
}

fn shell_command_from_input(input: &str) -> Option<String> {
    let command = input.strip_prefix('!')?.trim();
    (!command.is_empty()).then(|| command.to_string())
}

async fn run_shell_command(command: String) -> anyhow::Result<String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let output = tokio::time::timeout(
        Duration::from_secs(120),
        Command::new(shell).arg("-lc").arg(&command).output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("timed out after 120s"))??;
    let mut text = String::new();
    if !output.stdout.is_empty() {
        text.push_str(&String::from_utf8_lossy(&output.stdout));
    }
    if !output.stderr.is_empty() {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&String::from_utf8_lossy(&output.stderr));
    }
    let text = text.trim_end();
    let text = if text.is_empty() { "(no output)" } else { text };
    let text = truncate_for_transcript(text, 8_000);
    if output.status.success() {
        Ok(text)
    } else {
        Err(anyhow::anyhow!(
            "exit status {}: {}",
            output
                .status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "signal".to_string()),
            text
        ))
    }
}

fn truncate_for_transcript(text: &str, max_chars: usize) -> String {
    let mut truncated = text.chars().take(max_chars).collect::<String>();
    if truncated.len() < text.len() {
        truncated.push_str("\n... output truncated ...");
    }
    truncated
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

fn provider_choices_from_list(list: &ProvidersListResult) -> Vec<ProviderChoice> {
    list.providers
        .iter()
        .map(provider_choice_from_descriptor)
        .collect()
}

fn provider_choice_from_descriptor(provider: &ProviderDescriptor) -> ProviderChoice {
    ProviderChoice {
        provider_id: provider.id.clone(),
        name: provider.name.clone(),
        description: provider.description.clone(),
        auth_type: provider.auth_type.clone(),
        authenticated: provider.authenticated,
        auth_detail: provider.auth_detail.clone(),
        default_model: provider.models.first().map(|model| model.id.clone()),
        recommended: provider.recommended,
    }
}

fn main_provider_menu_items(providers: &[ProviderChoice]) -> Vec<ProviderMenuItem> {
    std::iter::once(ProviderMenuItem::Models)
        .chain(providers.iter().cloned().map(ProviderMenuItem::Provider))
        .collect()
}

fn filter_provider_menu_items(items: &[ProviderMenuItem], query: &str) -> Vec<ProviderMenuItem> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return items.to_vec();
    }
    items
        .iter()
        .filter(|item| item.label().to_lowercase().contains(&query))
        .cloned()
        .collect()
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

async fn session_get(client: &LocalAppClient) -> anyhow::Result<SessionGetResult> {
    let res = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("session/get")),
            method: "session/get".to_string(),
            params: None,
        })
        .await;
    decode_response(res)
}

async fn sessions_list(
    client: &LocalAppClient,
) -> anyhow::Result<Vec<roder_api::session::SessionMetadata>> {
    let res = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("sessions/list")),
            method: "sessions/list".to_string(),
            params: None,
        })
        .await;
    Ok(decode_response::<SessionsListResult>(res)?.sessions)
}

async fn agents_list(client: &LocalAppClient) -> anyhow::Result<Vec<AgentDescriptor>> {
    let res = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("agents/list")),
            method: "agents/list".to_string(),
            params: None,
        })
        .await;
    Ok(decode_response::<AgentsListResult>(res)?.agents)
}

async fn commands_list(client: &LocalAppClient) -> anyhow::Result<Vec<CommandDescriptor>> {
    let res = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("commands/list")),
            method: "commands/list".to_string(),
            params: None,
        })
        .await;
    Ok(decode_response::<CommandsListResult>(res)?.commands)
}

async fn commands_expand(
    client: &LocalAppClient,
    name: &str,
    arguments: &str,
) -> anyhow::Result<CommandsExpandResult> {
    let res = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("commands/expand")),
            method: "commands/expand".to_string(),
            params: Some(
                serde_json::to_value(CommandsExpandParams {
                    name: name.to_string(),
                    arguments: Some(arguments.to_string()),
                })
                .unwrap(),
            ),
        })
        .await;
    decode_response(res)
}

fn next_policy_mode(mode: PolicyMode) -> PolicyMode {
    match mode {
        PolicyMode::Default => PolicyMode::AcceptEdits,
        PolicyMode::AcceptEdits => PolicyMode::Plan,
        PolicyMode::Plan | PolicyMode::Bypass => PolicyMode::Default,
    }
}

fn policy_mode_label(mode: PolicyMode) -> &'static str {
    match mode {
        PolicyMode::Default => "default",
        PolicyMode::AcceptEdits => "accept_edits",
        PolicyMode::Plan => "plan",
        PolicyMode::Bypass => "bypass",
    }
}

fn truncate(value: &str, max_chars: usize) -> String {
    let mut out = value.chars().take(max_chars).collect::<String>();
    if out.len() < value.len() {
        out.push_str("...");
    }
    out
}

fn message_line(message: &str, theme: Theme) -> Line<'static> {
    if let Some(body) = message.strip_prefix("user: ") {
        let mut spans = vec![
            Span::styled("│ ", theme.accent()),
            Span::styled("you ", theme.accent()),
        ];
        spans.extend(linked_body_spans(body, theme.text(), theme.accent_soft()));
        return Line::from(spans);
    }
    if let Some(body) = message.strip_prefix("assistant: ") {
        let mut spans = vec![
            Span::styled("│ ", theme.accent_soft()),
            Span::styled("roder ", theme.strong()),
        ];
        spans.extend(linked_body_spans(body, theme.text(), theme.accent_soft()));
        return Line::from(spans);
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
    if let Some(body) = message.strip_prefix("command preview: ") {
        return Line::from(vec![
            Span::styled("↳ ", theme.subtle()),
            Span::styled(body.to_string(), theme.muted()),
        ]);
    }
    if let Some(body) = message.strip_prefix("shell: ") {
        let mut spans = vec![Span::styled("$ ", theme.tool())];
        spans.extend(linked_body_spans(body, theme.text(), theme.tool()));
        return Line::from(spans);
    }
    if let Some(body) = message.strip_prefix("shell output: ") {
        let mut spans = vec![Span::styled("↳ ", theme.subtle())];
        spans.extend(linked_body_spans(body, theme.muted(), theme.tool()));
        return Line::from(spans);
    }
    if let Some(body) = message.strip_prefix("system: ") {
        return Line::from(vec![
            Span::styled("• ", theme.subtle()),
            Span::styled(body.to_string(), theme.muted()),
        ]);
    }
    Line::from(Span::styled(message.to_string(), theme.text()))
}

fn linked_body_spans(body: &str, default_style: Style, link_style: Style) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut cursor = 0usize;
    for link in link_spans(body) {
        if link.start > body.len()
            || link.end > body.len()
            || link.start >= link.end
            || !body.is_char_boundary(link.start)
            || !body.is_char_boundary(link.end)
        {
            continue;
        }
        if cursor < link.start {
            spans.push(Span::styled(
                body[cursor..link.start].to_string(),
                default_style,
            ));
        }
        spans.push(Span::styled(
            body[link.start..link.end].to_string(),
            link_style.add_modifier(Modifier::UNDERLINED),
        ));
        cursor = link.end;
    }
    if cursor < body.len() {
        spans.push(Span::styled(body[cursor..].to_string(), default_style));
    }
    spans
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

fn interactive_region_id(event: &InteractiveEvent) -> Option<&str> {
    match event {
        InteractiveEvent::HoverEnter { region }
        | InteractiveEvent::HoverLeave { region }
        | InteractiveEvent::Click { region, .. }
        | InteractiveEvent::DoubleClick { region, .. }
        | InteractiveEvent::RightClick { region, .. }
        | InteractiveEvent::DragStart { region, .. }
        | InteractiveEvent::DragUpdate { region, .. }
        | InteractiveEvent::DragEnd { region, .. } => Some(region),
        InteractiveEvent::Scroll { region, .. } => region.as_deref(),
    }
}

fn region_kind_name(kind: &RegionKind) -> &'static str {
    match kind {
        RegionKind::TranscriptMessage { .. } => "TranscriptMessage",
        RegionKind::ToolCallBlock { .. } => "ToolCallBlock",
        RegionKind::FileReference { .. } => "FileReference",
        RegionKind::Url(_) => "Url",
        RegionKind::AttachmentThumbnail { .. } => "AttachmentThumbnail",
        RegionKind::StatusSegment { .. } => "StatusSegment",
        RegionKind::PaletteItem { .. } => "PaletteItem",
        RegionKind::DiffHunk { .. } => "DiffHunk",
        RegionKind::PolicyApprovalButton { .. } => "PolicyApprovalButton",
        RegionKind::Composer => "Composer",
        RegionKind::Custom { .. } => "Custom",
    }
}

fn mouse_capture_event_label(event: MouseCaptureEvent) -> &'static str {
    match event {
        MouseCaptureEvent::CaptureEnabled => "mouse capture enabled",
        MouseCaptureEvent::CaptureDisabled => "mouse capture disabled",
    }
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

fn current_git_snapshot() -> Option<GitSnapshot> {
    let output = std::process::Command::new("git")
        .args(["branch", "--show-current"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Some(GitSnapshot {
        branch: (!branch.is_empty()).then_some(branch),
    })
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
                theme.dialog_bg,
                theme.dialog_shadow,
                theme.dialog_key_bg,
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
    fn policy_mode_switcher_cycles_non_bypass_modes() {
        assert_eq!(
            next_policy_mode(PolicyMode::Default),
            PolicyMode::AcceptEdits
        );
        assert_eq!(next_policy_mode(PolicyMode::AcceptEdits), PolicyMode::Plan);
        assert_eq!(next_policy_mode(PolicyMode::Plan), PolicyMode::Default);
        assert_eq!(next_policy_mode(PolicyMode::Bypass), PolicyMode::Default);
    }

    #[test]
    fn policy_mode_labels_match_protocol_values() {
        assert_eq!(policy_mode_label(PolicyMode::Default), "default");
        assert_eq!(policy_mode_label(PolicyMode::AcceptEdits), "accept_edits");
        assert_eq!(policy_mode_label(PolicyMode::Plan), "plan");
        assert_eq!(policy_mode_label(PolicyMode::Bypass), "bypass");
    }

    #[test]
    fn provider_options_include_provider_models() {
        let list = ProvidersListResult {
            active_provider: "mock".to_string(),
            active_model: "mock".to_string(),
            providers: vec![ProviderDescriptor {
                id: "mock".to_string(),
                name: "Mock".to_string(),
                description: Some("Local".to_string()),
                auth_type: ProviderAuthType::None,
                auth_label: None,
                authenticated: true,
                auth_detail: None,
                recommended: false,
                sort_order: 100,
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
        let items = main_provider_menu_items(&[]);
        assert!(matches!(items.first(), Some(ProviderMenuItem::Models)));
    }

    #[test]
    fn provider_menu_filter_matches_labels_case_insensitively() {
        let items = main_provider_menu_items(&[ProviderChoice {
            provider_id: "codex".to_string(),
            name: "Codex".to_string(),
            description: Some("ChatGPT account provider".to_string()),
            auth_type: ProviderAuthType::OAuth,
            authenticated: false,
            auth_detail: None,
            default_model: Some("gpt-5.5".to_string()),
            recommended: true,
        }]);
        let filtered = filter_provider_menu_items(&items, "CODEX");
        assert_eq!(filtered.len(), 1);
        assert!(
            filtered
                .iter()
                .all(|item| item.label().to_lowercase().contains("codex"))
        );
    }

    #[test]
    fn provider_menu_filter_keeps_models_submenu_searchable() {
        let items = vec![
            ProviderMenuItem::Models,
            ProviderMenuItem::Model(ProviderOption {
                provider_id: "codex".to_string(),
                model_id: "gpt-5.5".to_string(),
                label: "codex / gpt-5.5 (GPT-5.5)".to_string(),
            }),
        ];
        let filtered = filter_provider_menu_items(&items, "5.5");
        assert_eq!(filtered.len(), 1);
        assert!(matches!(filtered[0], ProviderMenuItem::Model(_)));
    }

    #[test]
    fn shell_command_requires_non_empty_bang_prefix() {
        assert_eq!(
            shell_command_from_input("!echo hi").as_deref(),
            Some("echo hi")
        );
        assert_eq!(
            shell_command_from_input("!  echo hi  ").as_deref(),
            Some("echo hi")
        );
        assert_eq!(shell_command_from_input("!"), None);
        assert_eq!(shell_command_from_input("echo hi"), None);
    }

    #[test]
    fn message_line_formats_shell_messages() {
        let theme = Theme::for_dark_background(true);
        let line = message_line("shell: !echo hi", theme);
        let rendered = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert_eq!(rendered, "$ !echo hi");
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
