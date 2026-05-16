mod composer;
mod dialog;

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crossterm::{
    event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use roder_api::events::RoderEvent;
use roder_api::inference::ProviderAuthType;
use roder_api::policy_mode::{PolicyDecision, PolicyMode};
use roder_app_server::LocalAppClient;
use roder_protocol::{
    CodexAuthResult, CreateSessionResult, InterruptTurnParams, JsonRpcRequest, JsonRpcResponse,
    PendingPlanExitDescriptor, ProviderDescriptor, ProviderSelectParams, ProviderSelectResult,
    ProvidersListResult, SessionExitPlanParams, SessionExitPlanResult, SessionGetResult,
    SessionResolveApprovalParams, SessionResolveApprovalResult, SessionSetModeParams,
    SessionSetModeResult, StartTurnParams,
};
use tokio::process::Command;
use tui_textarea::TextArea;

use composer::{
    composer_mode, composer_text, composer_textarea, shell_command_from_input,
    style_composer_for_current_mode,
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
    shell: Color,
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
                shell: Color::Indexed(220),
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
            shell: Color::Indexed(160),
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

    fn shell(self) -> Style {
        Style::default().fg(self.shell).add_modifier(Modifier::BOLD)
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct ImageAttachment {
    path: PathBuf,
}

impl ImageAttachment {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn label(&self) -> String {
        self.path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_string)
            .unwrap_or_else(|| self.path.display().to_string())
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum ProviderPopupScreen {
    Main,
    Models,
}

#[derive(Debug, Clone, Eq, PartialEq)]
enum ConfirmDialog {
    Interrupt,
    Exit,
    ToolApproval {
        approval_id: String,
        tool_name: String,
        reason: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum ConfirmChoice {
    Yes,
    No,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct ConfirmDialogState {
    dialog: ConfirmDialog,
    selected: ConfirmChoice,
}

impl ConfirmDialogState {
    fn new(dialog: ConfirmDialog) -> Self {
        Self {
            dialog,
            selected: ConfirmChoice::Yes,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum ConfirmKeyAction {
    Confirm,
    Cancel,
    Select(ConfirmChoice),
    Ignore,
}

fn confirm_action_for_key(key: KeyCode, selected: ConfirmChoice) -> ConfirmKeyAction {
    match key {
        KeyCode::Left => ConfirmKeyAction::Select(ConfirmChoice::Yes),
        KeyCode::Right => ConfirmKeyAction::Select(ConfirmChoice::No),
        KeyCode::Enter if selected == ConfirmChoice::Yes => ConfirmKeyAction::Confirm,
        KeyCode::Enter => ConfirmKeyAction::Cancel,
        KeyCode::Char('y') | KeyCode::Char('Y') => ConfirmKeyAction::Confirm,
        KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => ConfirmKeyAction::Cancel,
        _ => ConfirmKeyAction::Ignore,
    }
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
    show_provider_popup: bool,
    provider_popup_screen: ProviderPopupScreen,
    provider_choices: Vec<ProviderChoice>,
    model_options: Vec<ProviderOption>,
    provider_menu_items: Vec<ProviderMenuItem>,
    provider_menu_filter: String,
    provider_state: ListState,
    confirm_dialog: Option<ConfirmDialogState>,
    image_attachments: Vec<ImageAttachment>,
    tool_call_names: HashMap<String, String>,
    policy_mode: PolicyMode,
    pending_plan_exit: Option<PendingPlanExitDescriptor>,
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
        let theme = Theme::for_terminal();
        let policy_state = session_get(&client).await.ok();

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
            show_provider_popup: false,
            provider_popup_screen: ProviderPopupScreen::Main,
            provider_choices: Vec::new(),
            model_options: Vec::new(),
            provider_menu_items: Vec::new(),
            provider_menu_filter: String::new(),
            provider_state,
            confirm_dialog: None,
            image_attachments: Vec::new(),
            tool_call_names: HashMap::new(),
            policy_mode: policy_state
                .as_ref()
                .map(|state| state.mode)
                .unwrap_or_default(),
            pending_plan_exit: policy_state.and_then(|state| state.pending_plan_exit),
            theme,
        })
    }

    pub async fn run(&mut self) -> anyhow::Result<()> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let mut rx = self.client.subscribe_events();

        loop {
            self.animation_frame = self.animation_frame.wrapping_add(1);
            terminal.draw(|f| self.render(f))?;

            if event::poll(Duration::from_millis(50))? {
                match event::read()? {
                    Event::Key(key) => {
                        if self.confirm_dialog.is_some() {
                            if self.handle_confirm_key(key).await {
                                break;
                            }
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
                        } else if key.modifiers.contains(KeyModifiers::CONTROL)
                            && key.code == KeyCode::Char('c')
                        {
                            self.confirm_dialog =
                                Some(ConfirmDialogState::new(ConfirmDialog::Exit));
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
                        } else {
                            match key.code {
                                KeyCode::Enter => {
                                    if key.modifiers.contains(KeyModifiers::SHIFT) {
                                        self.composer.insert_newline();
                                        continue;
                                    }
                                    self.submit_prompt().await;
                                }
                                KeyCode::Esc => {
                                    self.confirm_dialog = if self.active_turn_id.is_some() {
                                        Some(ConfirmDialogState::new(ConfirmDialog::Interrupt))
                                    } else {
                                        Some(ConfirmDialogState::new(ConfirmDialog::Exit))
                                    };
                                }
                                KeyCode::Backspace
                                    if composer_text(&self.composer).is_empty()
                                        && !self.image_attachments.is_empty() =>
                                {
                                    if let Some(attachment) = self.image_attachments.pop() {
                                        self.push_event(format!(
                                            "detached image {}",
                                            attachment.label()
                                        ));
                                    }
                                }
                                _ => {
                                    self.composer.input(key);
                                }
                            }
                        }
                    }
                    Event::Paste(text) => self.handle_paste(text),
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
                        if self.active_turn_id.as_deref() == Some(&ev.turn_id) {
                            self.active_turn_id = None;
                        }
                        self.messages.push(format!("error: {}", ev.error))
                    }
                    RoderEvent::ToolCallRequested(ev) => {
                        self.tool_call_names
                            .insert(ev.tool_id.clone(), ev.tool_name.clone());
                        self.messages
                            .push(format!("tool: {} requested", ev.tool_name));
                    }
                    RoderEvent::PolicyDecisionRecorded(ev) => match ev.decision {
                        PolicyDecision::Denied { reason } => {
                            self.record_error(format!("tool {} denied: {reason}", ev.tool_name));
                        }
                        PolicyDecision::RequiresApproval { .. } => {
                            self.messages
                                .push(format!("tool: {} awaiting approval", ev.tool_name));
                        }
                        PolicyDecision::Allowed | PolicyDecision::AutoApproved { .. } => {}
                    },
                    RoderEvent::ApprovalRequested(ev) => {
                        self.messages
                            .push(format!("tool: {} needs approval", ev.tool_name));
                        self.confirm_dialog =
                            Some(ConfirmDialogState::new(ConfirmDialog::ToolApproval {
                                approval_id: ev.approval_id,
                                tool_name: ev.tool_name,
                                reason: ev.reason,
                            }));
                    }
                    RoderEvent::ApprovalResolved(ev) => {
                        self.messages.push(format!(
                            "tool: {} approval {}",
                            ev.tool_name,
                            if ev.approved { "accepted" } else { "rejected" }
                        ));
                    }
                    RoderEvent::ToolCallCompleted(ev) => {
                        let name = self
                            .tool_call_names
                            .remove(&ev.tool_id)
                            .unwrap_or_else(|| format!("tool {}", short_id(&ev.tool_id)));
                        self.messages.push(format!("tool: {name} completed"));
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
                    _ => {}
                }
            }
        }

        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            DisableBracketedPaste,
            LeaveAlternateScreen
        )?;
        terminal.show_cursor()?;

        Ok(())
    }

    async fn handle_confirm_key(&mut self, key: crossterm::event::KeyEvent) -> bool {
        let Some(mut state) = self.confirm_dialog.clone() else {
            return false;
        };
        match confirm_action_for_key(key.code, state.selected) {
            ConfirmKeyAction::Select(choice) => {
                state.selected = choice;
                self.confirm_dialog = Some(state);
            }
            ConfirmKeyAction::Confirm => {
                self.confirm_dialog = None;
                match state.dialog {
                    ConfirmDialog::Interrupt => self.interrupt_active_turn().await,
                    ConfirmDialog::Exit => return true,
                    ConfirmDialog::ToolApproval { approval_id, .. } => {
                        self.resolve_tool_approval(approval_id, true).await
                    }
                }
            }
            ConfirmKeyAction::Cancel => {
                if let ConfirmDialog::ToolApproval { approval_id, .. } = state.dialog {
                    self.resolve_tool_approval(approval_id, false).await;
                }
                self.confirm_dialog = None;
            }
            ConfirmKeyAction::Ignore => {}
        }
        false
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

    async fn resolve_tool_approval(&mut self, approval_id: String, approved: bool) {
        let params = SessionResolveApprovalParams {
            approval_id: approval_id.clone(),
            approved,
        };
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("session/resolve_approval")),
                method: "session/resolve_approval".to_string(),
                params: Some(serde_json::to_value(params).unwrap()),
            })
            .await;
        match decode_response::<SessionResolveApprovalResult>(res) {
            Ok(result) if result.resolved => {}
            Ok(_) => self.record_error(format!("approval not pending: {}", short_id(&approval_id))),
            Err(err) => self.record_error(format!("session/resolve_approval failed: {err}")),
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
        let params = SessionSetModeParams {
            mode: next,
            reason: Some("tui mode switcher".to_string()),
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

    async fn submit_prompt(&mut self) {
        let text = composer_text(&self.composer).trim().to_string();
        let attachments = std::mem::take(&mut self.image_attachments);
        self.composer = composer_textarea(self.theme);
        if text.is_empty() && attachments.is_empty() {
            return;
        }
        if attachments.is_empty() && text.starts_with('/') {
            self.messages
                .push(format!("executed slash command: {text}"));
            return;
        }
        if attachments.is_empty()
            && let Some(command) = shell_command_from_input(&text)
        {
            self.run_shell_command(command).await;
            return;
        }

        let message = message_with_image_attachments(&text, &attachments);
        self.messages.push(format!(
            "user: {}",
            transcript_message_with_image_attachments(&text, &attachments)
        ));
        let params = StartTurnParams {
            thread_id: self.thread_id.clone(),
            message,
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

    fn handle_paste(&mut self, text: String) {
        if self.confirm_dialog.is_some() {
            return;
        }
        if self.show_provider_popup {
            self.provider_menu_filter
                .push_str(&text.replace(['\r', '\n'], " "));
            self.clamp_provider_menu_selection();
            return;
        }
        if let Some(attachments) = image_attachments_from_paste(&text) {
            let mut added = 0usize;
            for attachment in attachments {
                if !self
                    .image_attachments
                    .iter()
                    .any(|existing| existing.path == attachment.path)
                {
                    self.image_attachments.push(attachment);
                    added += 1;
                }
            }
            if added > 0 {
                self.push_event(format!("attached {added} image{}", plural_s(added)));
            }
            return;
        }
        self.composer.set_yank_text(text);
        self.composer.paste();
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
        style_composer_for_current_mode(&mut self.composer, self.theme);
        let event_height = event_log_height(self.show_event_log, self.events.len());
        let loader_height = if self.active_turn_id.is_some() { 1 } else { 0 };
        let attachment_height = image_attachment_height(self.image_attachments.len());
        let composer_height = self.composer.measure(area.width).preferred_rows;
        let mut constraints = Vec::new();
        if loader_height > 0 {
            constraints.push(Constraint::Length(loader_height));
        }
        constraints.extend([Constraint::Length(1), Constraint::Min(5)]);
        if event_height > 0 {
            constraints.push(Constraint::Length(event_height));
        }
        if attachment_height > 0 {
            constraints.push(Constraint::Length(attachment_height));
        }
        constraints.extend([Constraint::Length(composer_height), Constraint::Length(1)]);

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
        f.render_widget(self.header(area.width), chunks[header_index]);
        f.render_widget(
            self.transcript(chunks[transcript_index].height),
            chunks[transcript_index],
        );

        let mut composer_index = if event_height > 0 {
            f.render_widget(self.event_log(), chunks[transcript_index + 1]);
            transcript_index + 2
        } else {
            transcript_index + 1
        };
        if attachment_height > 0 {
            f.render_widget(self.image_attachment_bar(), chunks[composer_index]);
            composer_index += 1;
        }
        f.render_widget(&self.composer, chunks[composer_index]);
        f.render_widget(self.footer(area.width), chunks[composer_index + 1]);

        if self.show_provider_popup {
            self.render_provider_popup(f, area);
        }
        if let Some(dialog) = self.confirm_dialog.clone() {
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

    fn transcript(&self, height: u16) -> Paragraph<'static> {
        let lines = if self.messages.is_empty() {
            vec![
                Line::raw(""),
                Line::from(Span::styled(
                    "No transcript yet. Ask Roder to inspect, edit, or run something.",
                    self.theme.muted().add_modifier(Modifier::ITALIC),
                )),
            ]
        } else {
            self.messages
                .iter()
                .flat_map(|message| {
                    let mut lines = message_lines(message, self.theme);
                    lines.push(Line::raw(""));
                    lines
                })
                .collect::<Vec<_>>()
        };

        let scroll = transcript_scroll_offset(lines.len(), height);
        Paragraph::new(Text::from(lines))
            .style(self.theme.text())
            .scroll((scroll, 0))
            .wrap(Wrap { trim: false })
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

        Paragraph::new(text)
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(self.theme.border())
                    .title(Span::styled(" events ", self.theme.muted())),
            )
            .wrap(Wrap { trim: false })
    }

    fn image_attachment_bar(&self) -> Paragraph<'static> {
        let hidden = self.image_attachments.len().saturating_sub(3);
        let mut lines = self
            .image_attachments
            .iter()
            .rev()
            .take(3)
            .rev()
            .map(|attachment| {
                Line::from(vec![
                    Span::styled("image ", self.theme.accent_soft()),
                    Span::styled(attachment.label(), self.theme.text()),
                    Span::styled(
                        format!("  {}", attachment.path.display()),
                        self.theme.muted(),
                    ),
                ])
            })
            .collect::<Vec<_>>();
        if hidden > 0 {
            lines.push(Line::from(Span::styled(
                format!("+{hidden} more image{}", plural_s(hidden)),
                self.theme.muted(),
            )));
        }
        Paragraph::new(Text::from(lines))
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(self.theme.border())
                    .title(Span::styled(" attached images ", self.theme.muted())),
            )
            .wrap(Wrap { trim: false })
    }

    fn footer(&self, width: u16) -> Paragraph<'static> {
        let status = if self.active_turn_id.is_some() {
            "running"
        } else {
            "ready"
        };
        let pending_hint = self
            .pending_plan_exit
            .as_ref()
            .map(|pending| {
                let summary = pending.plan_summary.as_deref().unwrap_or("plan exit");
                format!(
                    "  exit plan? y approve / n reject: {}",
                    truncate(summary, 36)
                )
            })
            .unwrap_or_default();
        let shell_hint = if composer_mode(&self.composer).is_shell() {
            "  shell mode"
        } else {
            ""
        };
        Paragraph::new(line_with_gap(
            vec![Span::styled(
                format!(
                    " {status}  mode:{}{pending_hint}{shell_hint}  enter send  shift+enter newline  shift+tab mode  paste/drag images  ! shell  ctrl+p provider/model  ctrl+l events  esc interrupt  ctrl+c exit",
                    policy_mode_label(self.policy_mode)
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

    fn render_confirm_dialog(&self, f: &mut Frame<'_>, area: Rect, dialog: ConfirmDialogState) {
        dialog::render_confirm_dialog(f, area, dialog, self.theme);
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

fn transcript_scroll_offset(line_count: usize, height: u16) -> u16 {
    line_count.saturating_sub(usize::from(height)) as u16
}

fn image_attachment_height(count: usize) -> u16 {
    if count == 0 {
        0
    } else {
        (count as u16 + 1).clamp(2, 4)
    }
}

fn image_attachments_from_paste(text: &str) -> Option<Vec<ImageAttachment>> {
    let tokens = shell_like_tokens(text);
    if tokens.is_empty() {
        return None;
    }
    let mut attachments = Vec::new();
    for token in tokens {
        let path = image_path_from_token(&token)?;
        if !attachments
            .iter()
            .any(|existing: &ImageAttachment| existing.path == path)
        {
            attachments.push(ImageAttachment::new(path));
        }
    }
    (!attachments.is_empty()).then_some(attachments)
}

fn image_path_from_token(token: &str) -> Option<PathBuf> {
    let path = if let Some(uri) = token.strip_prefix("file://") {
        PathBuf::from(percent_decode(uri)?)
    } else {
        expand_home_path(token)
    };
    is_image_path(&path).then_some(path)
}

fn is_image_path(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|ext| ext.to_str()) else {
        return false;
    };
    let image_ext = matches!(
        ext.to_ascii_lowercase().as_str(),
        "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "webp"
            | "bmp"
            | "tif"
            | "tiff"
            | "heic"
            | "heif"
            | "avif"
            | "svg"
    );
    if !image_ext {
        return false;
    }
    path.is_absolute()
        || path.exists()
        || path.starts_with(".")
        || path.starts_with("..")
        || path.to_string_lossy().starts_with('~')
}

fn shell_like_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut chars = text.trim().chars().peekable();
    while let Some(ch) = chars.next() {
        match (ch, quote) {
            ('\\', _) => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            ('\'' | '"', None) => quote = Some(ch),
            ('\'' | '"', Some(active)) if active == ch => quote = None,
            (ch, None) if ch.is_whitespace() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn percent_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hi = bytes.get(i + 1).copied()?;
            let lo = bytes.get(i + 2).copied()?;
            decoded.push(hex_value(hi)? * 16 + hex_value(lo)?);
            i += 3;
        } else {
            decoded.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn expand_home_path(value: &str) -> PathBuf {
    if let Some(stripped) = value.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(stripped);
    }
    PathBuf::from(value)
}

fn message_with_image_attachments(text: &str, attachments: &[ImageAttachment]) -> String {
    let mut message = text.to_string();
    if attachments.is_empty() {
        return message;
    }
    if !message.is_empty() {
        message.push_str("\n\n");
    }
    message.push_str("Attached image file paths:");
    for attachment in attachments {
        message.push_str("\n- ");
        message.push_str(&attachment.path.display().to_string());
    }
    message
}

fn transcript_message_with_image_attachments(
    text: &str,
    attachments: &[ImageAttachment],
) -> String {
    if attachments.is_empty() {
        return text.to_string();
    }
    let image_labels = attachments
        .iter()
        .map(ImageAttachment::label)
        .collect::<Vec<_>>()
        .join(", ");
    if text.is_empty() {
        format!(
            "attached image{}: {image_labels}",
            plural_s(attachments.len())
        )
    } else {
        format!(
            "{text}\nattached image{}: {image_labels}",
            plural_s(attachments.len())
        )
    }
}

fn plural_s(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
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

#[cfg(test)]
fn message_line(message: &str, theme: Theme) -> Line<'static> {
    let mut lines = message_lines(message, theme);
    if lines.is_empty() {
        return Line::raw("");
    }
    lines.remove(0)
}

fn message_lines(message: &str, theme: Theme) -> Vec<Line<'static>> {
    if let Some(body) = message.strip_prefix("user: ") {
        return role_message_lines("", "", body, theme.accent(), theme.accent(), theme.text());
    }
    if let Some(body) = message.strip_prefix("assistant: ") {
        return role_message_lines(
            "",
            "",
            body,
            theme.accent_soft(),
            theme.strong(),
            theme.text(),
        );
    }
    if let Some(body) = message.strip_prefix("error: ") {
        return simple_message_lines("! ", body, theme.error(), theme.error());
    }
    if let Some(body) = message.strip_prefix("executed slash command: ") {
        return simple_message_lines("/ ", body, theme.tool(), theme.muted());
    }
    if let Some(body) = message.strip_prefix("tool: ") {
        return simple_message_lines("◆ ", body, theme.tool(), theme.muted());
    }
    if let Some(body) = message.strip_prefix("shell: ") {
        return simple_message_lines("$ ", body, theme.tool(), theme.text());
    }
    if let Some(body) = message.strip_prefix("shell output: ") {
        return simple_message_lines("↳ ", body, theme.subtle(), theme.muted());
    }
    if let Some(body) = message.strip_prefix("system: ") {
        return simple_message_lines("• ", body, theme.subtle(), theme.muted());
    }
    body_lines(message)
        .map(|line| Line::from(Span::styled(line, theme.text())))
        .collect()
}

fn role_message_lines(
    marker: &'static str,
    label: &'static str,
    body: &str,
    marker_style: Style,
    label_style: Style,
    body_style: Style,
) -> Vec<Line<'static>> {
    body_lines(body)
        .enumerate()
        .map(|(index, line)| {
            if index == 0 {
                return Line::from(vec![
                    Span::styled(marker, marker_style),
                    Span::styled(label, label_style),
                    Span::styled(line, body_style),
                ]);
            }
            let continuation_marker = if marker.is_empty() { "" } else { "  " };
            Line::from(vec![
                Span::styled(continuation_marker, marker_style),
                Span::styled(line, body_style),
            ])
        })
        .collect()
}

fn simple_message_lines(
    marker: &'static str,
    body: &str,
    marker_style: Style,
    body_style: Style,
) -> Vec<Line<'static>> {
    body_lines(body)
        .enumerate()
        .map(|(index, line)| {
            let marker = if index == 0 { marker } else { "  " };
            Line::from(vec![
                Span::styled(marker, marker_style),
                Span::styled(line, body_style),
            ])
        })
        .collect()
}

fn body_lines(body: &str) -> impl Iterator<Item = String> + '_ {
    body.split('\n').map(str::to_string)
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
                theme.shell,
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
    fn confirm_dialog_defaults_to_yes_to_preserve_enter_confirm() {
        let state = ConfirmDialogState::new(ConfirmDialog::Interrupt);

        assert_eq!(state.selected, ConfirmChoice::Yes);
        assert_eq!(
            confirm_action_for_key(KeyCode::Enter, state.selected),
            ConfirmKeyAction::Confirm
        );
    }

    #[test]
    fn confirm_dialog_arrow_keys_select_yes_and_no() {
        assert_eq!(
            confirm_action_for_key(KeyCode::Left, ConfirmChoice::No),
            ConfirmKeyAction::Select(ConfirmChoice::Yes)
        );
        assert_eq!(
            confirm_action_for_key(KeyCode::Right, ConfirmChoice::Yes),
            ConfirmKeyAction::Select(ConfirmChoice::No)
        );
        assert_eq!(
            confirm_action_for_key(KeyCode::Enter, ConfirmChoice::No),
            ConfirmKeyAction::Cancel
        );
    }

    #[test]
    fn confirm_dialog_keeps_y_and_n_quickbinds() {
        assert_eq!(
            confirm_action_for_key(KeyCode::Char('y'), ConfirmChoice::No),
            ConfirmKeyAction::Confirm
        );
        assert_eq!(
            confirm_action_for_key(KeyCode::Char('n'), ConfirmChoice::Yes),
            ConfirmKeyAction::Cancel
        );
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
        assert_eq!(rendered, "hello");
    }

    #[test]
    fn message_lines_preserve_multiline_assistant_output() {
        let lines = message_lines(
            "assistant: Created files:\n\n- `Package.swift`",
            Theme::for_dark_background(true),
        );
        let rendered = lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert_eq!(rendered, vec!["Created files:", "", "- `Package.swift`"]);
    }

    #[test]
    fn message_lines_format_tool_messages() {
        let lines = message_lines(
            "tool: write_file completed",
            Theme::for_dark_background(true),
        );
        let rendered = lines[0]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert_eq!(rendered, "◆ write_file completed");
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
    fn transcript_scroll_offset_follows_latest_lines() {
        assert_eq!(transcript_scroll_offset(3, 10), 0);
        assert_eq!(transcript_scroll_offset(10, 10), 0);
        assert_eq!(transcript_scroll_offset(14, 10), 4);
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
    fn image_attachment_height_only_allocates_when_images_are_attached() {
        assert_eq!(image_attachment_height(0), 0);
        assert_eq!(image_attachment_height(1), 2);
        assert_eq!(image_attachment_height(3), 4);
        assert_eq!(image_attachment_height(10), 4);
    }

    #[test]
    fn image_paste_detects_absolute_and_escaped_image_paths() {
        let attachments = image_attachments_from_paste("/tmp/first.png /tmp/second\\ image.jpg")
            .expect("expected image attachments");

        assert_eq!(attachments.len(), 2);
        assert_eq!(attachments[0].path, PathBuf::from("/tmp/first.png"));
        assert_eq!(attachments[1].path, PathBuf::from("/tmp/second image.jpg"));
    }

    #[test]
    fn image_paste_detects_file_uris() {
        let attachments = image_attachments_from_paste("file:///tmp/Screen%20Shot.webp")
            .expect("expected image attachment");

        assert_eq!(attachments[0].path, PathBuf::from("/tmp/Screen Shot.webp"));
    }

    #[test]
    fn image_paste_ignores_mixed_text() {
        assert!(image_attachments_from_paste("look at /tmp/image.png please").is_none());
    }

    #[test]
    fn prompt_message_includes_attached_image_paths() {
        let attachments = vec![ImageAttachment::new(PathBuf::from("/tmp/diagram.png"))];

        assert_eq!(
            message_with_image_attachments("what is this?", &attachments),
            "what is this?\n\nAttached image file paths:\n- /tmp/diagram.png"
        );
        assert_eq!(
            transcript_message_with_image_attachments("", &attachments),
            "attached image: diagram.png"
        );
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
    fn composer_mode_tracks_shell_prefix() {
        assert_eq!(
            composer::composer_mode_from_text("!echo hi"),
            composer::ComposerMode::Shell
        );
        assert_eq!(
            composer::composer_mode_from_text("echo hi"),
            composer::ComposerMode::Chat
        );
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
