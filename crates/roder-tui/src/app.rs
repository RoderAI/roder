#[allow(dead_code)]
mod automations;
mod commands;
mod composer;
mod dialog;
mod goals;
mod input_queue;
mod media;
#[allow(dead_code)]
mod memories;
#[cfg(test)]
mod plan_hunk_tests;
mod plan_panel;
mod plan_review;
mod plugin_browser;
mod processes;
#[allow(dead_code)]
mod remote;
mod roadmap_workspace;
mod runner;
mod scroll_accel;
mod shortcuts;
#[allow(dead_code)]
mod skills;
mod stream_animation;
mod subagent_trace;
#[cfg(test)]
mod subagent_trace_tests;
#[allow(dead_code)]
mod team_panes;
mod team_ui;
mod thread_resume;
mod tool_detail;
mod tool_timeline;
mod turn_timer;
mod voice;
mod webwright;
mod workflow_import;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
#[cfg(test)]
use crossterm::event::KeyboardEnhancementFlags;
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::{
    Frame,
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Widget, Wrap,
    },
};
use roder_api::catalog::lookup_model;
use roder_api::events::RoderEvent;
use roder_api::inference::{
    HostedWebSearchMode, ProviderAuthType, ReasoningEffortDescriptor, TokenUsage,
};
use roder_api::policy_mode::{PolicyDecision, PolicyMode};
use roder_api::transcript::InputImage;
use roder_api_transcript::ApiTranscriptRecord;
use roder_app_server::{
    AppClient, AppEventReceiver, AppServer, LocalAppClient, transcript::TranscriptRecorder,
};
use roder_protocol::{
    AgentsListResult, CommandDescriptor, CommandsExpandParams, CommandsExpandResult,
    CommandsListResult, JsonRpcRequest, JsonRpcResponse, PendingPlanExitDescriptor,
    ProviderAuthResult, ProviderClearParams, ProviderClearResult, ProviderConfigureParams,
    ProviderConfigureResult, ProviderDescriptor, ProviderSelectParams, ProviderSelectResult,
    ProvidersListResult, RunnersListResult, RunnersSelectParams, RunnersSelectResult,
    SettingsGetResult, SettingsSetDefaultModeParams, SettingsSetDefaultModeResult,
    SettingsSetFileBackedDynamicContextParams, SettingsSetFileBackedDynamicContextResult,
    SettingsSetSearchIndexParams, SettingsSetSearchIndexResult, SettingsSetShellParams,
    SettingsSetShellResult, SettingsSetWebSearchParams, SettingsSetWebSearchResult, ShellSettings,
    SpeechProvidersListResult, TasksGetParams, TasksGetResult, TasksListResult, TeamReadParams,
    TeamReadResult, Thread, ThreadExitPlanParams, ThreadExitPlanResult, ThreadGoal,
    ThreadResolveApprovalParams, ThreadResolveApprovalResult, ThreadSetModeParams,
    ThreadSetModeResult, ThreadStartParams, ThreadStartResult, ThreadStateResult, TurnInputItem,
    TurnInterruptParams, TurnStartParams, TurnSteerParams,
};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tui_textarea::TextArea;

use self::commands::built_in_command_catalog;
use crate::frame_snapshot;
use crate::roadmap::RoadmapModeState;
#[cfg(test)]
use crate::runtime_io::keyboard_enhancement_flags;
use crate::runtime_io::{
    CrosstermInputSource, RecordingInputSource, SystemClock, TerminalSession, TuiClock,
    TuiInputRecorder, TuiInputSource,
};
use composer::{
    ComposerKeyAction, composer_mode, composer_text, composer_textarea, handle_composer_key,
    shell_command_from_input, style_composer_for_current_mode,
};
use input_queue::{PendingPrompt, PromptQueue, queue_status};
use plan_panel::{
    PlanPanelState, plan_counter_area, plan_panel_height, render_plan_counter, render_plan_panel,
};
use plugin_browser::PluginBrowserState;
use remote::{RemotePanelController, render_remote_panel_lines};
use roadmap_workspace::{RoadmapWorkspaceMeta, render_roadmap_workspace};
use roder_roadmap::ThreadAttachment;
use scroll_accel::ScrollSettings;
use shortcuts::FooterShortcutContext;
use team_ui::{TeamUiState, is_team_focus_next_key, is_team_focus_previous_key};
use tool_detail::{ToolDetailAction, ToolDetailModal, render_tool_detail_modal};
use tool_timeline::{
    TimelineFocus, TimelineSettings, TimelineState, ToolTimelineEntry, TurnCompletedSummary,
    fallback_entry,
};
use turn_timer::TurnTimer;
use voice::{VoiceConfig, VoiceMode, VoiceState};

const TOP_STATUS_ANIMATION_FPS: u64 = 6;
const MAX_VISIBLE_SLASH_COMMANDS: usize = 16;
const COPIED_HELPER_LABEL: &str = "Copied to clipboard";
const COPIED_HELPER_DURATION: Duration = Duration::from_secs(2);

#[derive(Clone, Default)]
pub struct TuiRunOptions {
    pub transcript_recorder: Option<TranscriptRecorder>,
    pub record_ui_frames: bool,
}

#[derive(Clone)]
struct OptionalInputRecorder {
    recorder: Option<TranscriptRecorder>,
}

impl TuiInputRecorder for OptionalInputRecorder {
    fn record_input(&mut self, input: roder_api_transcript::RecordedUiInput) -> anyhow::Result<()> {
        let Some(recorder) = &self.recorder else {
            return Ok(());
        };
        let (seq, at_ms) = recorder.next_seq_at_ms();
        recorder.push(ApiTranscriptRecord::UiInput {
            seq,
            at_ms,
            event: input,
        })
    }
}

fn should_handle_key_event(key: KeyEvent) -> bool {
    matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat)
}

fn pending_turn_input(text: String, images: Vec<InputImage>) -> Vec<TurnInputItem> {
    let mut input = Vec::new();
    if !text.is_empty() {
        input.push(TurnInputItem {
            kind: "text".to_string(),
            text: Some(text),
            path: None,
            image_url: None,
        });
    }
    input.extend(images.into_iter().map(|image| TurnInputItem {
        kind: "image".to_string(),
        text: None,
        path: None,
        image_url: Some(image.image_url),
    }));
    input
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Theme {
    text: Color,
    text_strong: Color,
    commentary: Color,
    muted: Color,
    subtle: Color,
    accent: Color,
    accent_soft: Color,
    tool: Color,
    tool_running: Color,
    diff_added: Color,
    diff_added_bg: Color,
    diff_removed: Color,
    diff_removed_bg: Color,
    diff_line_number: Color,
    shell: Color,
    error: Color,
    border: Color,
    mode_default: Color,
    mode_accept_all: Color,
    mode_plan: Color,
    mode_bypass: Color,
    dialog: Color,
    dialog_bg: Color,
    dialog_shadow: Color,
    dialog_key_bg: Color,
    selection_fg: Color,
    selection_bg: Color,
    /// If true, renderers skip nodes whose CSS class would resolve to
    /// `display: none`. The CSS engine populates this from the active
    /// stylesheet — see `crate::theme::overrides::ThemeOverrides::hides`.
    pub hide_thinking: bool,
    /// Optional fill for the entire frame. `None` means the theme is
    /// transparent and the terminal's native background bleeds through —
    /// this is the default and matches `:root { background: transparent }`.
    /// Themes set a concrete color via `:root { background: ... }`,
    /// `:root { --background: ... }`, or `#body { background-color: ... }`.
    pub body_background: Option<Color>,
    /// Border shape applied to every framed widget (composer, popup, dialog,
    /// tool detail, palette, diff). Themes set this via
    /// `:root { border-radius: 0 }`, `:root { border-style: rounded }`, or
    /// `#body { border: double }`.
    pub border_type: BorderType,
    /// When the theme requests `border: none` / `border-style: none`, framed
    /// widgets render their inner area only and skip drawing the box.
    pub borders_visible: bool,
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
                commentary: Color::Indexed(15),
                muted: Color::Indexed(244),
                subtle: Color::Indexed(245),
                accent: Color::Indexed(212),
                accent_soft: Color::Indexed(183),
                tool: Color::Indexed(244),
                tool_running: Color::Indexed(244),
                diff_added: Color::Indexed(114),
                diff_added_bg: Color::Indexed(22),
                diff_removed: Color::Indexed(210),
                diff_removed_bg: Color::Indexed(52),
                diff_line_number: Color::Indexed(246),
                shell: Color::Indexed(220),
                error: Color::Indexed(196),
                border: Color::Indexed(244),
                mode_default: Color::Indexed(244),
                mode_accept_all: Color::Indexed(40),
                mode_plan: Color::Indexed(75),
                mode_bypass: Color::Indexed(196),
                dialog: Color::Indexed(62),
                dialog_bg: Color::Indexed(235),
                dialog_shadow: Color::Indexed(232),
                dialog_key_bg: Color::Indexed(238),
                selection_fg: Color::Reset,
                selection_bg: Color::Indexed(212),
                hide_thinking: false,
                body_background: None,
                border_type: BorderType::Rounded,
                borders_visible: true,
            };
        }

        Self {
            text: Color::Reset,
            text_strong: Color::Reset,
            commentary: Color::Indexed(16),
            muted: Color::Indexed(240),
            subtle: Color::Indexed(240),
            accent: Color::Indexed(198),
            accent_soft: Color::Indexed(96),
            tool: Color::Indexed(240),
            tool_running: Color::Indexed(240),
            diff_added: Color::Indexed(28),
            diff_added_bg: Color::Indexed(194),
            diff_removed: Color::Indexed(160),
            diff_removed_bg: Color::Indexed(224),
            diff_line_number: Color::Indexed(244),
            shell: Color::Indexed(160),
            error: Color::Indexed(160),
            border: Color::Indexed(240),
            mode_default: Color::Indexed(240),
            mode_accept_all: Color::Indexed(28),
            mode_plan: Color::Indexed(25),
            mode_bypass: Color::Indexed(160),
            dialog: Color::Indexed(62),
            dialog_bg: Color::Indexed(255),
            dialog_shadow: Color::Indexed(250),
            dialog_key_bg: Color::Indexed(252),
            selection_fg: Color::Reset,
            selection_bg: Color::Indexed(198),
            hide_thinking: false,
            body_background: None,
            border_type: BorderType::Rounded,
            borders_visible: true,
        }
    }

    /// Patch fields from a parsed CSS theme. Variables that the theme does not
    /// declare leave the baseline untouched. Hidden classes (`display: none`)
    /// flip the matching renderer flags. This is the proof's stand-in for full
    /// per-node cascade — see `crates/roder-tui/src/theme/overrides.rs`.
    fn apply_overrides(mut self, overrides: &crate::theme::ThemeOverrides) -> Self {
        macro_rules! set {
            ($field:ident, $var:literal) => {
                if let Some(c) = overrides.color($var) {
                    self.$field = c;
                }
            };
        }
        set!(text, "text");
        set!(text_strong, "text");
        set!(commentary, "commentary");
        set!(muted, "muted");
        set!(subtle, "subtle");
        set!(accent, "accent");
        set!(accent_soft, "accent-soft");
        set!(tool, "tool");
        set!(tool_running, "tool");
        set!(diff_added, "diff-added");
        set!(diff_added_bg, "diff-added-bg");
        set!(diff_removed, "diff-removed");
        set!(diff_removed_bg, "diff-removed-bg");
        set!(shell, "shell");
        set!(error, "error");
        set!(border, "border");
        set!(mode_plan, "mode-plan");
        set!(mode_default, "mode-default");
        set!(mode_accept_all, "mode-accept-all");
        set!(mode_bypass, "mode-bypass");
        set!(selection_bg, "selection-bg");
        set!(selection_fg, "selection-fg");
        set!(dialog, "dialog");
        // NB: `dialog-bg` / `dialog-shadow` are deliberately *not* honored
        // here even when set by the theme — see the auto-sync block below.
        // The popup interior always matches the body so the popup reads as
        // a framed cutout of the same surface, not as an elevated card.
        // `dialog-key-bg` (hotkey chips on confirm dialogs) stays themable
        // because those chips need contrast against the dialog body.
        set!(dialog_key_bg, "dialog-key-bg");
        if overrides.hides("timeline-thinking") {
            self.hide_thinking = true;
        }
        // `None` from overrides means the theme is transparent — leave the
        // baseline (also `None`) so the terminal's own background shows.
        if overrides.background.is_some() {
            self.body_background = overrides.background;
        }
        // Popup interior + shadow always mirror the body. Transparent body
        // (`body_background == None`) resolves to `Color::Reset` so popups
        // render against the terminal's native background.
        let body_or_reset = self.body_background.unwrap_or(Color::Reset);
        self.dialog_bg = body_or_reset;
        self.dialog_shadow = body_or_reset;
        if let Some(shape) = overrides.border_shape {
            use roder_theme::BorderShape;
            match shape {
                BorderShape::None => {
                    self.borders_visible = false;
                }
                BorderShape::Plain => {
                    self.borders_visible = true;
                    self.border_type = BorderType::Plain;
                }
                BorderShape::Rounded => {
                    self.borders_visible = true;
                    self.border_type = BorderType::Rounded;
                }
                BorderShape::Double => {
                    self.borders_visible = true;
                    self.border_type = BorderType::Double;
                }
                BorderShape::Thick => {
                    self.borders_visible = true;
                    self.border_type = BorderType::Thick;
                }
            }
        }
        self
    }

    /// Same as [`Self::for_terminal`] but layers any active CSS theme found in
    /// the user's `~/.roder/themes/` directory, the project-local
    /// `.roder/themes/` directory, or the repo's `themes/` directory.
    fn for_terminal_themed() -> Self {
        let base = Self::for_terminal();
        match crate::theme::load_active_theme(&crate::theme::discovery::default_directories(), None)
        {
            Some(overrides) => base.apply_overrides(&overrides),
            None => base,
        }
    }

    /// Public-within-crate handle so the palette can re-apply a freshly loaded
    /// override set without touching `apply_overrides`'s private signature.
    /// The `palette_ui` submodule is the only consumer today; it isn't wired
    /// into the live input loop yet, so allow dead_code while the picker work
    /// continues.
    #[allow(dead_code)]
    pub(crate) fn with_overrides(self, overrides: &crate::theme::ThemeOverrides) -> Self {
        self.apply_overrides(overrides)
    }

    fn text(self) -> Style {
        Style::default().fg(self.text)
    }

    fn strong(self) -> Style {
        Style::default()
            .fg(self.text_strong)
            .add_modifier(Modifier::BOLD)
    }

    fn commentary(self) -> Style {
        Style::default().fg(self.commentary)
    }

    fn muted(self) -> Style {
        Style::default().fg(self.muted)
    }

    fn subtle(self) -> Style {
        Style::default().fg(self.subtle)
    }

    fn user_surface(self) -> Style {
        Style::default().fg(self.text_strong)
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

    fn diff_added(self) -> Style {
        Style::default().fg(self.diff_added).bg(self.diff_added_bg)
    }

    fn diff_removed(self) -> Style {
        Style::default()
            .fg(self.diff_removed)
            .bg(self.diff_removed_bg)
    }

    fn diff_line_number(self) -> Style {
        Style::default().fg(self.diff_line_number)
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

    fn policy_mode(self, mode: PolicyMode) -> Style {
        let color = match mode {
            PolicyMode::Default => self.mode_default,
            PolicyMode::AcceptAll => self.mode_accept_all,
            PolicyMode::Plan => self.mode_plan,
            PolicyMode::Bypass => self.mode_bypass,
        };
        Style::default().fg(color).add_modifier(Modifier::BOLD)
    }

    fn dialog(self) -> Style {
        // Border glyphs (├ ┐ ╭ ┴ ...) should blend with the body fill so the
        // popup frame looks like it floats on the body, not stamped with the
        // terminal default. If the theme is transparent (`body_background ==
        // None`) we leave the bg unset so true-transparent terminals stay
        // transparent through the border cells too.
        let mut style = Style::default().fg(self.dialog);
        if let Some(bg) = self.body_background {
            style = style.bg(bg);
        }
        style
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
    context_window: Option<u32>,
    default_reasoning: Option<String>,
    reasoning_options: Vec<ReasoningEffortDescriptor>,
}

#[derive(Debug, Clone)]
struct ReasoningOptionChoice {
    provider_id: String,
    model_id: String,
    effort: String,
    description: String,
}

#[derive(Debug, Clone)]
struct VoiceModelChoice {
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

#[derive(Debug, Clone, Copy)]
struct ProviderAuthFlow {
    provider_id: &'static str,
    display_name: &'static str,
    login_method: &'static str,
    logout_method: &'static str,
}

impl ProviderAuthFlow {
    fn for_provider(provider_id: &str) -> Option<Self> {
        match provider_id {
            "codex" => Some(Self {
                provider_id: "codex",
                display_name: "Codex",
                login_method: "auth/codex/login",
                logout_method: "auth/codex/logout",
            }),
            "supergrok" => Some(Self {
                provider_id: "supergrok",
                display_name: "SuperGrok",
                login_method: "auth/supergrok/login",
                logout_method: "auth/supergrok/logout",
            }),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ImageAttachment {
    path: PathBuf,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct SelectionPoint {
    row: u16,
    column: u16,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct MouseSelection {
    anchor: SelectionPoint,
    cursor: SelectionPoint,
    dragging: bool,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct CopiedHelper {
    shown_at: Instant,
}

impl CopiedHelper {
    fn visible(self, now: Instant) -> bool {
        now.duration_since(self.shown_at) < COPIED_HELPER_DURATION
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct SelectableLine {
    row: u16,
    text: String,
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
    Providers,
    ApiKey,
    Models,
    Reasoning,
    Settings,
    Runners,
    Spinner,
    WebSearch,
    VoiceModels,
    Shell,
    Resume,
    Themes,
    Marketplaces,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum WorkingSpinner {
    Dots,
    Line,
    Arc,
    Pulse,
}

impl WorkingSpinner {
    fn all() -> &'static [Self] {
        &[Self::Dots, Self::Line, Self::Arc, Self::Pulse]
    }

    fn id(self) -> &'static str {
        match self {
            Self::Dots => "dots",
            Self::Line => "line",
            Self::Arc => "arc",
            Self::Pulse => "pulse",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Dots => "Dots",
            Self::Line => "Line",
            Self::Arc => "Arc",
            Self::Pulse => "Pulse",
        }
    }

    fn frames(self) -> &'static [&'static str] {
        match self {
            Self::Dots => &[".", "..", "...", " ..", "  .", "   "],
            Self::Line => &["-", "\\", "|", "/"],
            Self::Arc => &["(", "(.", "(.)", ".)", ")", " "],
            Self::Pulse => &[".", "o", "O", "o"],
        }
    }

    fn from_config(config: Option<&str>) -> Self {
        config
            .and_then(|value| {
                Self::all()
                    .iter()
                    .copied()
                    .find(|spinner| spinner.id() == value)
            })
            .unwrap_or(Self::Dots)
    }
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

fn is_policy_mode_switch_key(key: KeyEvent) -> bool {
    key.code == KeyCode::BackTab
}

fn is_plan_panel_toggle_key(key: KeyEvent) -> bool {
    key.code == KeyCode::Char('t') && key.modifiers.contains(KeyModifiers::CONTROL)
}

fn is_raw_tool_name(name: &str) -> bool {
    !name.trim().is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
}

fn confirm_dialog_allows_policy_switch(state: &ConfirmDialogState) -> bool {
    matches!(state.dialog, ConfirmDialog::ToolApproval { .. })
}

fn tool_approval_dialog_matches(state: &ConfirmDialogState, approval_id: &str) -> bool {
    matches!(
        &state.dialog,
        ConfirmDialog::ToolApproval {
            approval_id: current,
            ..
        } if current == approval_id
    )
}

fn is_dialog_menu_previous_key(key: KeyEvent) -> bool {
    key.code == KeyCode::Up
        || (key.code == KeyCode::Char('k') && key.modifiers.contains(KeyModifiers::CONTROL))
}

fn is_dialog_menu_next_key(key: KeyEvent) -> bool {
    key.code == KeyCode::Down
        || (key.code == KeyCode::Char('j') && key.modifiers.contains(KeyModifiers::CONTROL))
}

#[derive(Debug, Clone)]
enum ProviderMenuItem {
    Section(String),
    Models,
    Providers,
    Settings,
    RoadmapMode,
    RunnerSettings,
    SpinnerSettings,
    WebSearchSettings,
    VoiceModelSettings,
    ShellSettings(String),
    SearchIndexToggle(bool),
    FileBackedDynamicContextToggle(bool),
    MessageFoldingToggle(bool),
    ThemesSettings,
    MarketplacesSettings,
    PluginBrowser,
    ResumeThreads,
    DefaultMode(PolicyMode),
    Spinner(WorkingSpinner),
    WebSearchMode(HostedWebSearchMode),
    VoiceModel(VoiceModelChoice),
    ShellChoice(String),
    Provider(ProviderChoice),
    Model(ProviderOption),
    Reasoning(ReasoningOptionChoice),
    Runner {
        destination_id: String,
        provider_id: String,
        label: String,
    },
    Thread(Box<Thread>),
    Theme(String),
    MarketplaceDefault {
        id: &'static str,
        kind: &'static str,
        label: &'static str,
    },
    MarketplaceInstallDefault {
        selection: &'static str,
        label: &'static str,
    },
    Back,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum ActiveTurnPromptShortcut {
    Queue,
    Steer,
}

fn active_turn_prompt_shortcut(
    key: KeyEvent,
    has_prepared_prompt: bool,
) -> Option<ActiveTurnPromptShortcut> {
    if !has_prepared_prompt {
        return None;
    }
    match key.code {
        KeyCode::Tab => Some(ActiveTurnPromptShortcut::Queue),
        KeyCode::Enter if key.modifiers == KeyModifiers::NONE => {
            Some(ActiveTurnPromptShortcut::Steer)
        }
        _ => None,
    }
}

fn composer_queue_key(key: KeyEvent, has_prepared_prompt: bool) -> bool {
    has_prepared_prompt && key.modifiers == KeyModifiers::NONE && key.code == KeyCode::Tab
}

impl ProviderMenuItem {
    fn label(&self) -> String {
        match self {
            Self::Section(label) => label.clone(),
            Self::Models => "Models".to_string(),
            Self::Providers => "Providers".to_string(),
            Self::Settings => "Settings".to_string(),
            Self::RoadmapMode => "Roadmaps".to_string(),
            Self::RunnerSettings => "Runners".to_string(),
            Self::SpinnerSettings => "Working spinner".to_string(),
            Self::WebSearchSettings => "Web search provider".to_string(),
            Self::VoiceModelSettings => "Voice model".to_string(),
            Self::ShellSettings(shell) => format!("Shell command shell: {shell}"),
            Self::SearchIndexToggle(enabled) => format!(
                "Instant regex search: {}",
                if *enabled { "on" } else { "off" }
            ),
            Self::FileBackedDynamicContextToggle(enabled) => format!(
                "File-backed dynamic context: {}",
                if *enabled { "on" } else { "off" }
            ),
            Self::MessageFoldingToggle(enabled) => format!(
                "Fold long messages: {}",
                if *enabled { "on" } else { "off" }
            ),
            Self::ThemesSettings => "Themes".to_string(),
            Self::MarketplacesSettings => "Plugin marketplaces".to_string(),
            Self::PluginBrowser => "Browse installable plugins".to_string(),
            Self::ResumeThreads => "Resume thread".to_string(),
            Self::DefaultMode(mode) => {
                format!("Default mode: {}", settings_policy_mode_label(*mode))
            }
            Self::Spinner(spinner) => spinner.label().to_string(),
            Self::WebSearchMode(mode) => web_search_mode_label(*mode).to_string(),
            Self::VoiceModel(choice) => choice.label.clone(),
            Self::ShellChoice(shell) => shell.clone(),
            Self::Provider(provider) => provider.label(),
            Self::Model(option) => option.label.clone(),
            Self::Reasoning(option) => format!("{} - {}", option.effort, option.description),
            Self::Runner { label, .. } => label.clone(),
            Self::Thread(thread) => {
                let workspace = if thread.cwd.trim().is_empty() {
                    "(unknown)".to_string()
                } else {
                    thread.cwd.clone()
                };
                format!(
                    "{} [{}] {}",
                    thread.updated_at,
                    short_id(&thread.id),
                    thread
                        .name
                        .clone()
                        .filter(|title| !title.trim().is_empty())
                        .unwrap_or_else(|| format!("Thread {}", short_id(&thread.id)))
                        + &format!(" - {workspace}")
                )
            }
            Self::Theme(id) => id.clone(),
            Self::MarketplaceDefault { id, kind, label } => format!("{label} - {kind} ({id})"),
            Self::MarketplaceInstallDefault { label, .. } => label.to_string(),
            Self::Back => "Back".to_string(),
        }
    }

    fn is_selectable(&self) -> bool {
        !matches!(self, Self::Section(_))
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
            ProviderAuthType::ApiKey if self.authenticated => {
                label.push_str(" - API key configured");
            }
            ProviderAuthType::ApiKey => {
                label.push_str(" - paste API key");
            }
            _ => {}
        }
        label
    }
}

pub struct TuiApp<C = LocalAppClient>
where
    C: AppClient,
{
    client: C,
    thread_id: String,
    thread_title: Option<String>,
    thread_message_count: usize,
    active_turn_id: Option<String>,
    active_turn_timer: TurnTimer,
    current_turn_input_tokens: u32,
    current_turn_output_tokens: u32,
    current_turn_reasoning_tokens: Option<u32>,
    current_turn_total_tokens: u32,
    thread_tokens: u64,
    context_window_tokens: u64,
    provider: String,
    model: String,
    model_context_window: Option<u32>,
    context_counter_hovered: bool,
    last_frame_width: u16,
    selectable_lines: Vec<SelectableLine>,
    mouse_selection: Option<MouseSelection>,
    copied_helper: Option<CopiedHelper>,
    reasoning_effort: String,
    composer: TextArea<'static>,
    timeline: TimelineState,
    team_ui: TeamUiState,
    team_timelines: HashMap<String, TimelineState>,
    plan_panel: PlanPanelState,
    tool_names: HashMap<String, String>,
    exec_session_tools: HashMap<u64, String>,
    stdin_tool_sessions: HashMap<String, u64>,
    hidden_stdin_tools: HashSet<String>,
    last_plan_counter_area: Option<Rect>,
    events: Vec<String>,
    animation_frame: u64,
    show_event_log: bool,
    show_provider_popup: bool,
    show_shortcuts_dialog: bool,
    provider_popup_screen: ProviderPopupScreen,
    provider_choices: Vec<ProviderChoice>,
    model_options: Vec<ProviderOption>,
    pending_reasoning_model: Option<ProviderOption>,
    pending_api_key_provider: Option<ProviderChoice>,
    provider_menu_items: Vec<ProviderMenuItem>,
    provider_menu_filter: String,
    provider_state: ListState,
    working_spinner: WorkingSpinner,
    scroll_settings: ScrollSettings,
    timeline_settings: TimelineSettings,
    web_search_mode: HostedWebSearchMode,
    search_index_enabled: bool,
    command_shell: String,
    command_shell_options: Vec<String>,
    file_backed_dynamic_context: bool,
    confirm_dialog: Option<ConfirmDialogState>,
    tool_detail_modal: Option<ToolDetailModal>,
    plugin_browser: Option<PluginBrowserState>,
    remote_panel: RemotePanelController,
    roadmap_mode: Option<RoadmapModeState>,
    image_attachments: Vec<ImageAttachment>,
    queued_prompts: PromptQueue,
    last_user_prompt: Option<PendingPrompt>,
    command_catalog: Vec<CommandDescriptor>,
    slash_command_selection: usize,
    voice: VoiceState,
    policy_mode: PolicyMode,
    pending_plan_exit: Option<PendingPlanExitDescriptor>,
    current_goal: Option<ThreadGoal>,
    compaction_active: bool,
    theme: Theme,
    /// Id of the currently-applied theme (basename of the `.css` file). `None`
    /// when running on the compiled-in baseline because no theme file was
    /// discoverable at startup. The palette's Themes source consults this to
    /// flag the active row. Read from `palette_ui::open_palette` (currently
    /// orphan code awaiting input-loop wiring).
    #[allow(dead_code)]
    pub(crate) active_theme_id: Option<String>,
    /// While the Themes submenu is open, this holds the `(theme,
    /// active_theme_id)` pair from before the user entered it. Each navigation
    /// in the submenu replaces `self.theme` with a live preview; `Esc` /
    /// `Back` restores from this snapshot, `Enter` commits and clears it.
    theme_preview_baseline: Option<(Theme, Option<String>)>,
}

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub enum TuiStartup {
    #[default]
    NewThread,
    ResumeMenu,
    ResumeThread(String),
    RoadmapOpen {
        path: Option<String>,
    },
    TeamAttach {
        team_id: String,
        member_id: String,
    },
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct TuiExitSummary {
    pub thread_id: String,
    pub title: String,
    pub model: String,
    pub message_count: usize,
    pub resume_command: String,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct ThreadParts {
    thread_id: String,
    provider: String,
    thread_model: String,
    requested_model: String,
    reasoning: String,
    thread_title: Option<String>,
    thread_message_count: usize,
}

#[derive(Debug, serde::Deserialize)]
struct RoadmapThreadResponse {
    thread: ThreadAttachment,
}

impl TuiApp<LocalAppClient> {
    pub async fn new(client: LocalAppClient, model: String) -> anyhow::Result<Self> {
        Self::new_with_startup(client, model, TuiStartup::NewThread).await
    }

    pub async fn new_with_startup(
        client: LocalAppClient,
        model: String,
        startup: TuiStartup,
    ) -> anyhow::Result<Self> {
        let remote_panel_server = client.app_server();
        TuiApp::new_with_startup_and_remote(client, model, startup, remote_panel_server).await
    }
}

impl<C> TuiApp<C>
where
    C: AppClient,
{
    pub async fn new_with_startup_and_remote(
        client: C,
        model: String,
        startup: TuiStartup,
        remote_panel_server: Arc<AppServer>,
    ) -> anyhow::Result<Self> {
        if let TuiStartup::TeamAttach { team_id, member_id } = startup.clone() {
            let team = team_read(&client, &team_id)
                .await?
                .team
                .ok_or_else(|| anyhow::anyhow!("team not found: {}", short_id(&team_id)))?;
            let member = team
                .members
                .iter()
                .find(|member| member.id == member_id)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("team member not found: {member_id}"))?;
            let thread = thread_resume::load_thread(&client, &member.thread_id).await?;
            let thread_model = member
                .model
                .clone()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| model.clone());
            let provider = member
                .model_provider
                .clone()
                .or_else(|| thread.as_ref().map(|thread| thread.model_provider.clone()))
                .unwrap_or_default();
            let title = Some(format!("{} ({})", member.name, short_id(&team.id)));
            let message_count = thread
                .as_ref()
                .and_then(|thread| thread.turns.as_ref())
                .map(|turns| {
                    turns
                        .iter()
                        .flat_map(|turn| turn.items.iter())
                        .filter(|item| matches!(item.kind.as_str(), "userMessage" | "agentMessage"))
                        .count()
                })
                .unwrap_or_default();
            let mut app = Self::from_thread_parts(
                client,
                remote_panel_server,
                ThreadParts {
                    thread_id: member.thread_id.clone(),
                    provider,
                    thread_model,
                    requested_model: String::new(),
                    reasoning: "medium".to_string(),
                    thread_title: title,
                    thread_message_count: message_count,
                },
            )
            .await?;
            app.team_ui.set_team(team.id, team.members);
            app.team_ui.focus_member(&member_id);
            app.load_focused_team_timeline();
            if let Some(thread) = thread {
                app.apply_thread(thread);
            }
            return Ok(app);
        }

        if let TuiStartup::ResumeThread(thread_id) = startup.clone() {
            let thread = thread_resume::load_thread(&client, &thread_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("thread not found: {}", short_id(&thread_id)))?;
            let provider = thread.model_provider.clone();
            let thread_model = model.clone();
            let title = thread
                .name
                .clone()
                .filter(|title| !title.trim().is_empty())
                .or_else(|| (!thread.preview.trim().is_empty()).then(|| thread.preview.clone()));
            let message_count = thread
                .turns
                .as_ref()
                .map(|turns| {
                    turns
                        .iter()
                        .flat_map(|turn| turn.items.iter())
                        .filter(|item| matches!(item.kind.as_str(), "userMessage" | "agentMessage"))
                        .count()
                })
                .unwrap_or_default();
            let mut app = Self::from_thread_parts(
                client,
                remote_panel_server,
                ThreadParts {
                    thread_id: thread_id.clone(),
                    provider,
                    thread_model,
                    requested_model: String::new(),
                    reasoning: "medium".to_string(),
                    thread_title: title,
                    thread_message_count: message_count,
                },
            )
            .await?;
            app.apply_thread(thread);
            return Ok(app);
        }

        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(1)),
            method: "thread/start".to_string(),
            params: Some(
                serde_json::to_value(ThreadStartParams {
                    model: (!model.trim().is_empty()).then(|| model.clone()),
                    model_provider: None,
                    cwd: std::env::current_dir()?.display().to_string(),
                    ephemeral: false,
                })
                .unwrap(),
            ),
        };

        let res = client.send_request(req).await;
        let started = if let Some(result) = res.result {
            serde_json::from_value::<ThreadStartResult>(result)?
        } else {
            anyhow::bail!("failed to create thread: {:?}", res.error);
        };

        let selected_model = if model.is_empty() {
            started.model.clone()
        } else {
            model.clone()
        };
        let mut app = Self::from_thread_parts(
            client,
            remote_panel_server,
            ThreadParts {
                thread_id: started.thread.id,
                provider: started.model_provider,
                thread_model: selected_model,
                requested_model: model,
                reasoning: "medium".to_string(),
                thread_title: None,
                thread_message_count: 0,
            },
        )
        .await?;

        match startup {
            TuiStartup::NewThread => {}
            TuiStartup::ResumeMenu => {
                app.open_resume_submenu().await;
            }
            TuiStartup::ResumeThread(thread_id) => {
                app.load_thread(thread_id).await;
            }
            TuiStartup::RoadmapOpen { path } => {
                app.enter_roadmap_mode(path);
            }
            TuiStartup::TeamAttach { .. } => {}
        }

        Ok(app)
    }

    async fn from_thread_parts(
        client: C,
        remote_panel_server: Arc<AppServer>,
        parts: ThreadParts,
    ) -> anyhow::Result<Self> {
        let ThreadParts {
            thread_id,
            provider,
            thread_model,
            requested_model,
            reasoning,
            thread_title,
            thread_message_count,
        } = parts;
        let mut provider_state = ListState::default();
        provider_state.select(Some(0));
        let theme = Theme::for_terminal_themed();
        let remote_panel = RemotePanelController::new(
            remote_panel_server,
            std::env::current_dir()
                .ok()
                .map(|path| path.display().to_string()),
        );
        // Mirror the resolution that for_terminal_themed performed so the
        // palette's Themes source can flag the active row consistently. If
        // discovery yields nothing we leave this as `None` and the palette
        // will show no row as active.
        let active_theme_id = {
            let dirs = crate::theme::discovery::default_directories();
            let entries = crate::theme::discover_themes(&dirs);
            crate::theme::discovery::active_theme(&entries, None).map(|e| e.id.clone())
        };
        let policy_state = thread_state(&client).await.ok();
        let settings_state = settings_get(&client).await.ok();
        let shell_settings = settings_state
            .as_ref()
            .map(|settings| settings.shell.clone())
            .unwrap_or_else(default_shell_settings);
        let tui_config = load_tui_config().unwrap_or_default();
        let selected_model = if requested_model.is_empty() {
            thread_model
        } else {
            requested_model
        };
        let model_context_window = context_window_for_model(&selected_model);

        let command_catalog = thread_resume::commands_list(&client)
            .await
            .map(commands::with_local_commands)
            .unwrap_or_else(|_| built_in_command_catalog());
        let current_goal = goals::thread_goal_get(&client, &thread_id)
            .await
            .ok()
            .and_then(|result| result.goal);
        let scroll_settings = tui_config.scroll_settings();
        let timeline_settings = tui_config.timeline_settings();

        Ok(Self {
            client,
            thread_id,
            thread_title,
            thread_message_count,
            active_turn_id: None,
            active_turn_timer: TurnTimer::default(),
            current_turn_input_tokens: 0,
            current_turn_output_tokens: 0,
            current_turn_reasoning_tokens: None,
            current_turn_total_tokens: 0,
            thread_tokens: 0,
            context_window_tokens: 0,
            provider,
            model: selected_model,
            model_context_window,
            context_counter_hovered: false,
            last_frame_width: 0,
            selectable_lines: Vec::new(),
            mouse_selection: None,
            copied_helper: None,
            reasoning_effort: reasoning,
            composer: composer_textarea(theme),
            timeline: TimelineState::new(scroll_settings, timeline_settings),
            team_ui: TeamUiState::default(),
            team_timelines: HashMap::new(),
            plan_panel: PlanPanelState::default(),
            tool_names: HashMap::new(),
            exec_session_tools: HashMap::new(),
            stdin_tool_sessions: HashMap::new(),
            hidden_stdin_tools: HashSet::new(),
            last_plan_counter_area: None,
            events: Vec::new(),
            animation_frame: 0,
            show_event_log: false,
            show_provider_popup: false,
            show_shortcuts_dialog: false,
            provider_popup_screen: ProviderPopupScreen::Main,
            provider_choices: Vec::new(),
            model_options: Vec::new(),
            pending_reasoning_model: None,
            pending_api_key_provider: None,
            provider_menu_items: Vec::new(),
            provider_menu_filter: String::new(),
            provider_state,
            working_spinner: WorkingSpinner::from_config(tui_config.spinner.as_deref()),
            scroll_settings,
            timeline_settings,
            web_search_mode: settings_state
                .as_ref()
                .map(|settings| settings.web_search.mode)
                .unwrap_or(HostedWebSearchMode::Cached),
            search_index_enabled: settings_state
                .as_ref()
                .map(|settings| settings.search_index.enabled)
                .unwrap_or(true),
            command_shell: shell_settings.shell,
            command_shell_options: shell_settings.options,
            file_backed_dynamic_context: settings_state
                .map(|settings| settings.file_backed_dynamic_context)
                .unwrap_or(true),
            confirm_dialog: None,
            tool_detail_modal: None,
            plugin_browser: None,
            remote_panel,
            roadmap_mode: None,
            image_attachments: Vec::new(),
            queued_prompts: PromptQueue::default(),
            last_user_prompt: None,
            command_catalog,
            slash_command_selection: 0,
            voice: VoiceState::from_config(tui_config.voice.clone().unwrap_or_default()),
            policy_mode: policy_state
                .as_ref()
                .map(|state| state.mode)
                .unwrap_or_default(),
            pending_plan_exit: policy_state.and_then(|state| state.pending_plan_exit),
            current_goal,
            compaction_active: false,
            theme,
            active_theme_id,
            theme_preview_baseline: None,
        })
    }

    pub fn enter_roadmap_mode(&mut self, path: Option<String>) {
        let label = path.clone().unwrap_or_else(|| "roadmap".to_string());
        let workspace = std::env::current_dir();
        let state = workspace
            .as_deref()
            .ok()
            .and_then(|workspace| RoadmapModeState::load(workspace, path.clone()).ok())
            .unwrap_or_else(|| RoadmapModeState::new(path));
        self.roadmap_mode = Some(state);
        self.push_event(format!("Roadmapping mode: {label}"));
    }

    pub fn exit_summary(&self) -> TuiExitSummary {
        self.thread_exit_summary()
    }

    pub async fn run(&mut self) -> anyhow::Result<()> {
        self.run_with_options(TuiRunOptions::default()).await
    }

    pub async fn run_with_options(&mut self, options: TuiRunOptions) -> anyhow::Result<()> {
        let mut session = TerminalSession::enter()?;
        let mut input = RecordingInputSource::new(
            CrosstermInputSource,
            OptionalInputRecorder {
                recorder: options.transcript_recorder.clone(),
            },
        );
        let clock = SystemClock;

        let mut rx = self.client.subscribe_events();
        let mut next_animation_tick = clock.now() + top_status_animation_interval();

        loop {
            let now = clock.now();
            advance_top_status_animation(&mut self.animation_frame, &mut next_animation_tick, now);
            self.tick_streaming_animations(now, session.terminal_mut().size()?.width);
            self.stop_idle_voice_recording(now).await;
            self.finish_voice_transcription_if_ready().await;
            session.terminal_mut().draw(|f| {
                self.render(f);
                if options.record_ui_frames
                    && let Some(recorder) = &options.transcript_recorder
                {
                    let frame = frame_snapshot::recorded_frame(f.buffer_mut(), true);
                    let (seq, at_ms) = recorder.next_seq_at_ms();
                    let _ = recorder.push(ApiTranscriptRecord::UiFrame { seq, at_ms, frame });
                }
            })?;

            if input.poll(self.animation_poll_timeout(next_animation_tick, clock.now()))? {
                match input.read()? {
                    Event::Key(key) => {
                        if self.handle_voice_key(key).await {
                            continue;
                        }
                        if !should_handle_key_event(key) {
                            continue;
                        }
                        if let Some(modal) = self.tool_detail_modal.as_mut() {
                            match modal.handle_key(key) {
                                ToolDetailAction::Close => self.tool_detail_modal = None,
                                ToolDetailAction::Handled => {}
                            }
                        } else if self.confirm_dialog_allows_policy_switch()
                            && is_policy_mode_switch_key(key)
                        {
                            self.cycle_policy_mode().await;
                        } else if self.confirm_dialog.is_some() {
                            if self.handle_confirm_key(key).await {
                                break;
                            }
                        } else if self.show_shortcuts_dialog {
                            if key.modifiers.contains(KeyModifiers::CONTROL)
                                && key.code == KeyCode::Char('c')
                            {
                                self.confirm_dialog =
                                    Some(ConfirmDialogState::new(ConfirmDialog::Exit));
                                self.show_shortcuts_dialog = false;
                            } else if shortcuts::shortcut_dialog_close_key(key) {
                                self.show_shortcuts_dialog = false;
                            }
                        } else if self.plugin_browser.is_some() {
                            if key.modifiers.contains(KeyModifiers::CONTROL)
                                && key.code == KeyCode::Char('c')
                            {
                                self.plugin_browser = None;
                                self.confirm_dialog =
                                    Some(ConfirmDialogState::new(ConfirmDialog::Exit));
                            } else {
                                self.handle_plugin_browser_key(key).await;
                            }
                        } else if key.modifiers.contains(KeyModifiers::CONTROL)
                            && key.code == KeyCode::Char('p')
                        {
                            if self.show_provider_popup {
                                // Toggling the popup off short-circuits the
                                // normal Esc path, so revert any in-progress
                                // theme preview here too.
                                self.cancel_theme_preview();
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
                        } else if is_plan_panel_toggle_key(key) {
                            self.toggle_plan_panel();
                        } else if key.modifiers.contains(KeyModifiers::CONTROL)
                            && key.code == KeyCode::Char('c')
                        {
                            self.confirm_dialog =
                                Some(ConfirmDialogState::new(ConfirmDialog::Exit));
                        } else if is_policy_mode_switch_key(key) {
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
                                KeyCode::Char('o')
                                    if self.provider_popup_screen
                                        == ProviderPopupScreen::ApiKey
                                        && key.modifiers.contains(KeyModifiers::CONTROL) =>
                                {
                                    self.open_provider_api_key_url().await;
                                }
                                KeyCode::Enter
                                    if self.provider_popup_screen
                                        == ProviderPopupScreen::ApiKey =>
                                {
                                    self.submit_provider_api_key().await;
                                }
                                KeyCode::Backspace
                                    if self.provider_popup_screen
                                        == ProviderPopupScreen::ApiKey =>
                                {
                                    self.provider_menu_filter.pop();
                                }
                                KeyCode::Char(c)
                                    if self.provider_popup_screen
                                        == ProviderPopupScreen::ApiKey
                                        && !key.modifiers.contains(KeyModifiers::CONTROL) =>
                                {
                                    self.provider_menu_filter.push(c);
                                }
                                KeyCode::Delete
                                    if self.provider_popup_screen
                                        == ProviderPopupScreen::Providers =>
                                {
                                    if let Some(selected) = self.provider_state.selected() {
                                        if let Some(ProviderMenuItem::Provider(provider)) = self
                                            .filtered_provider_menu_items()
                                            .get(selected)
                                            .cloned()
                                        {
                                            self.clear_or_logout_provider(provider).await;
                                        }
                                    }
                                }
                                KeyCode::Backspace
                                    if self.provider_popup_screen
                                        == ProviderPopupScreen::Providers
                                        && self.provider_menu_filter.is_empty() =>
                                {
                                    if let Some(selected) = self.provider_state.selected() {
                                        if let Some(ProviderMenuItem::Provider(provider)) = self
                                            .filtered_provider_menu_items()
                                            .get(selected)
                                            .cloned()
                                        {
                                            self.clear_or_logout_provider(provider).await;
                                        }
                                    }
                                }
                                _ if is_dialog_menu_previous_key(key) => {
                                    self.select_previous_provider_menu_item();
                                }
                                _ if is_dialog_menu_next_key(key) => {
                                    self.select_next_provider_menu_item();
                                }
                                KeyCode::Enter => self.select_current_provider_menu_item().await,
                                KeyCode::Backspace => {
                                    self.provider_menu_filter.pop();
                                    self.clamp_provider_menu_selection();
                                    self.preview_highlighted_theme();
                                }
                                KeyCode::Char(c)
                                    if !key.modifiers.contains(KeyModifiers::CONTROL) =>
                                {
                                    self.provider_menu_filter.push(c);
                                    self.clamp_provider_menu_selection();
                                    self.preview_highlighted_theme();
                                }
                                _ => {}
                            }
                        } else if self.roadmap_mode.is_some() && self.handle_roadmap_key(key).await
                        {
                            continue;
                        } else {
                            if is_team_focus_next_key(key) {
                                self.cycle_team_focus(true);
                                continue;
                            }
                            if is_team_focus_previous_key(key) {
                                self.cycle_team_focus(false);
                                continue;
                            }
                            if self.handle_slash_command_key(key).await {
                                continue;
                            }
                            if composer_queue_key(key, self.has_prepared_prompt()) {
                                self.queue_current_prompt();
                                continue;
                            }
                            if shortcuts::should_open_shortcuts_dialog(
                                key,
                                composer_text(&self.composer).trim().is_empty(),
                                self.timeline.focus() == TimelineFocus::Composer,
                            ) {
                                self.show_shortcuts_dialog = true;
                                continue;
                            }
                            if self.active_turn_id.is_some()
                                && let Some(shortcut) =
                                    active_turn_prompt_shortcut(key, self.has_prepared_prompt())
                            {
                                match shortcut {
                                    ActiveTurnPromptShortcut::Queue => {
                                        self.queue_current_prompt();
                                    }
                                    ActiveTurnPromptShortcut::Steer => {
                                        self.submit_prompt().await;
                                    }
                                }
                                continue;
                            }
                            if key.code == KeyCode::Up && self.can_edit_queued_prompt() {
                                self.edit_latest_queued_prompt();
                                continue;
                            }
                            if key.code == KeyCode::Tab {
                                self.timeline.focus_latest();
                                continue;
                            }
                            if self.timeline.is_focused() && self.timeline.handle_key(key) {
                                continue;
                            }
                            match key.code {
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
                                _ => match handle_composer_key(&mut self.composer, key) {
                                    ComposerKeyAction::Submit => self.submit_prompt().await,
                                    ComposerKeyAction::Edited => {
                                        self.slash_command_selection = 0;
                                        self.timeline.focus_composer();
                                    }
                                    ComposerKeyAction::Ignored => {
                                        self.timeline.focus_composer();
                                    }
                                },
                            }
                        }
                    }
                    Event::Paste(text) => self.handle_paste(text),
                    Event::Mouse(mouse) => self.handle_mouse(mouse),
                    _ => {}
                }
            }

            while let Ok(envelope) = rx.try_recv() {
                self.push_event(format!("{} #{}", envelope.kind, envelope.seq));
                self.remote_panel.apply_event(&envelope);

                match envelope.event {
                    RoderEvent::TurnStarted(ev) => {
                        self.active_turn_id = Some(ev.turn_id);
                        self.active_turn_timer.start(clock.now());
                        self.current_turn_input_tokens = 0;
                        self.current_turn_output_tokens = 0;
                        self.current_turn_reasoning_tokens = None;
                        self.current_turn_total_tokens = 0;
                        self.compaction_active = false;
                    }
                    RoderEvent::TurnCompleted(ev)
                        if self.active_turn_id.as_deref() == Some(&ev.turn_id) =>
                    {
                        self.flush_streaming_animation_for_thread(&ev.thread_id);
                        let elapsed = self.active_turn_timer.finish(clock.now());
                        self.active_turn_id = None;
                        self.timeline.push_turn_completed(TurnCompletedSummary {
                            elapsed,
                            input_tokens: self.current_turn_input_tokens,
                            output_tokens: self.current_turn_output_tokens,
                            reasoning_tokens: self.current_turn_reasoning_tokens,
                            thread_tokens: self.thread_tokens,
                        });
                        self.current_turn_input_tokens = 0;
                        self.current_turn_output_tokens = 0;
                        self.current_turn_reasoning_tokens = None;
                        self.current_turn_total_tokens = 0;
                        self.compaction_active = false;
                        self.submit_next_queued_prompt().await;
                    }
                    RoderEvent::TurnInterrupted(ev)
                        if self.active_turn_id.as_deref() == Some(&ev.turn_id) =>
                    {
                        self.flush_streaming_animation_for_thread(&ev.thread_id);
                        self.active_turn_id = None;
                        self.active_turn_timer.reset();
                        self.current_turn_input_tokens = 0;
                        self.current_turn_output_tokens = 0;
                        self.current_turn_reasoning_tokens = None;
                        self.current_turn_total_tokens = 0;
                        self.compaction_active = false;
                    }
                    RoderEvent::InferenceEventReceived(ev) => {
                        self.team_ui.record_thread_activity(&ev.thread_id);
                        match ev.event {
                            roder_api::inference::InferenceEvent::MessageDelta(delta) => {
                                if let Some(timeline) =
                                    self.team_timeline_for_thread_mut(&ev.thread_id)
                                {
                                    timeline
                                        .push_assistant_delta_streaming(&delta.text, delta.phase);
                                } else {
                                    self.timeline
                                        .push_assistant_delta_streaming(&delta.text, delta.phase);
                                }
                            }
                            roder_api::inference::InferenceEvent::ReasoningDelta(delta) => {
                                if let Some(timeline) =
                                    self.team_timeline_for_thread_mut(&ev.thread_id)
                                {
                                    timeline.push_reasoning_delta_streaming(&delta.text);
                                } else {
                                    self.timeline.push_reasoning_delta_streaming(&delta.text);
                                }
                            }
                            roder_api::inference::InferenceEvent::Usage(usage) => {
                                self.record_usage(usage);
                            }
                            roder_api::inference::InferenceEvent::ToolCallStarted(call) => {
                                self.record_tool_requested_with_id(
                                    call.id,
                                    fallback_entry(call.name),
                                );
                            }
                            roder_api::inference::InferenceEvent::ToolCallDelta(delta) => {
                                self.record_tool_delta(&delta.id, &delta.arguments_delta);
                            }
                            roder_api::inference::InferenceEvent::ToolCallCompleted(call) => {
                                self.record_tool_requested_with_id(
                                    call.id,
                                    ToolTimelineEntry::new(call.name, call.arguments),
                                );
                            }
                            roder_api::inference::InferenceEvent::HostedToolCallStarted(call) => {
                                self.record_tool_requested_with_id(
                                    call.id,
                                    fallback_entry(call.name),
                                );
                            }
                            roder_api::inference::InferenceEvent::HostedToolCallCompleted(call) => {
                                let tool_id = call.id.clone();
                                self.record_tool_requested_with_id(
                                    tool_id.clone(),
                                    ToolTimelineEntry::new(call.name, call.arguments),
                                );
                                self.record_tool_completed(&tool_id, false, None);
                            }
                            roder_api::inference::InferenceEvent::Compaction(compaction) => {
                                self.record_compaction_progress(&compaction.status);
                            }
                            roder_api::inference::InferenceEvent::ProviderMetadata(metadata) => {
                                self.record_provider_metadata(&metadata);
                            }
                            _ => {}
                        }
                    }
                    RoderEvent::TurnFailed(ev) => {
                        self.flush_streaming_animation_for_thread(&ev.thread_id);
                        if self.active_turn_id.as_deref() == Some(&ev.turn_id) {
                            self.active_turn_id = None;
                            self.active_turn_timer.reset();
                            self.current_turn_input_tokens = 0;
                            self.current_turn_output_tokens = 0;
                            self.current_turn_reasoning_tokens = None;
                            self.current_turn_total_tokens = 0;
                            self.compaction_active = false;
                        }
                        self.timeline.push_error(ev.error);
                    }
                    RoderEvent::ToolCallRequested(ev) => {
                        self.record_tool_requested_with_id(
                            ev.tool_id,
                            fallback_entry(ev.tool_name),
                        );
                    }
                    RoderEvent::PolicyDecisionRecorded(ev) => match ev.decision {
                        PolicyDecision::Denied { reason } => {
                            self.record_tool_completed(&ev.tool_id, true, None);
                            self.record_error(format!("tool {} denied: {reason}", ev.tool_name));
                        }
                        PolicyDecision::RequiresApproval { .. } => {
                            self.record_tool_requested_with_id(
                                ev.tool_id,
                                fallback_entry(format!("{} awaiting approval", ev.tool_name)),
                            );
                        }
                        PolicyDecision::Allowed | PolicyDecision::AutoApproved { .. } => {}
                    },
                    RoderEvent::ApprovalRequested(ev) => {
                        if self.active_turn_id.as_deref() == Some(&ev.turn_id) {
                            self.active_turn_timer.pause(clock.now());
                        }
                        self.record_tool_requested_with_id(
                            ev.tool_id,
                            fallback_entry(format!("{} needs approval", ev.tool_name)),
                        );
                        self.confirm_dialog =
                            Some(ConfirmDialogState::new(ConfirmDialog::ToolApproval {
                                approval_id: ev.approval_id,
                                tool_name: ev.tool_name,
                                reason: ev.reason,
                            }));
                    }
                    RoderEvent::ApprovalResolved(ev) => {
                        self.clear_tool_approval_dialog(&ev.approval_id);
                        if self.active_turn_id.as_deref() == Some(&ev.turn_id) {
                            self.active_turn_timer.resume(clock.now());
                        }
                        if !ev.approved {
                            self.record_tool_completed(&ev.tool_id, true, None);
                        }
                    }
                    RoderEvent::UserInputRequested(ev) => {
                        let question = ev
                            .questions
                            .get(0)
                            .and_then(|question| question.get("question"))
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("user input requested");
                        self.timeline
                            .push_system(format!("user input requested: {question}"));
                    }
                    RoderEvent::UserInputResolved(ev) => {
                        self.timeline.push_system(format!(
                            "user input resolved: {}",
                            short_id(&ev.request_id)
                        ));
                    }
                    RoderEvent::ToolCallCompleted(ev) => {
                        self.record_tool_completed(&ev.tool_id, ev.is_error, ev.output);
                    }
                    RoderEvent::ThreadGoalUpdated(ev) if ev.thread_id == self.thread_id => {
                        self.current_goal = Some(ev.goal);
                    }
                    RoderEvent::ThreadGoalCleared(ev) if ev.thread_id == self.thread_id => {
                        self.current_goal = None;
                    }
                    RoderEvent::ThreadGoalUpdated(_) | RoderEvent::ThreadGoalCleared(_) => {}
                    RoderEvent::SubagentTraceCreated(ev) => {
                        self.timeline.record_subagent_trace_created(ev.summary);
                    }
                    RoderEvent::SubagentTraceDelta(ev) => {
                        self.timeline.record_subagent_trace_delta(ev.delta);
                    }
                    RoderEvent::SubagentTraceStatusChanged(ev) => {
                        self.timeline.record_subagent_trace_status(
                            &ev.trace_id,
                            ev.status,
                            ev.detail,
                        );
                    }
                    RoderEvent::SubagentTraceCompleted(ev) => {
                        self.timeline.record_subagent_trace_completed(ev.summary);
                    }
                    RoderEvent::SubagentTraceFailed(ev) => {
                        self.timeline.record_subagent_trace_failed(ev.summary);
                    }
                    RoderEvent::PlanReviewCreated(ev) => {
                        self.timeline.record_plan_review_created(ev.review);
                    }
                    RoderEvent::PlanReviewStatusChanged(ev) => {
                        self.timeline
                            .record_plan_review_status(&ev.review_id, ev.status);
                    }
                    RoderEvent::PlanReviewCommentAdded(ev) => {
                        self.timeline.record_plan_review_comment(ev.comment);
                    }
                    RoderEvent::PlanReviewRewritten(ev) => {
                        self.timeline.record_plan_review_rewrite(ev.rewrite);
                    }
                    RoderEvent::PlanReviewApproved(ev) => {
                        self.timeline.record_plan_review_status(
                            &ev.review_id,
                            roder_api::plan_review::PlanReviewStatus::Approved,
                        );
                    }
                    RoderEvent::PlanReviewRejected(ev) => {
                        self.timeline.record_plan_review_status(
                            &ev.review_id,
                            roder_api::plan_review::PlanReviewStatus::Rejected,
                        );
                    }
                    RoderEvent::HunkRecorded(ev) => {
                        self.timeline.record_hunk(ev.hunk);
                    }
                    RoderEvent::TeamMemberStatusChanged(ev) => {
                        self.team_ui.set_member_status(&ev.member_id, ev.status);
                    }
                    RoderEvent::TeamMemberCompleted(ev) => {
                        self.team_ui.set_member_status(&ev.member_id, ev.status);
                    }
                    RoderEvent::PolicyModeChanged(ev) => {
                        self.policy_mode = ev.new_mode;
                        self.push_event(format!(
                            "policy mode changed: {}",
                            policy_mode_label(ev.new_mode)
                        ));
                    }
                    RoderEvent::PolicyExitPlanRequested(_) => {
                        self.refresh_thread_state().await;
                    }
                    RoderEvent::PolicyExitPlanResolved(_) => {
                        self.refresh_thread_state().await;
                    }
                    _ => {}
                }
            }
        }

        session.restore()?;

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

    fn confirm_dialog_allows_policy_switch(&self) -> bool {
        self.confirm_dialog
            .as_ref()
            .is_some_and(confirm_dialog_allows_policy_switch)
    }

    fn clear_tool_approval_dialog(&mut self, approval_id: &str) {
        let should_clear = self
            .confirm_dialog
            .as_ref()
            .is_some_and(|state| tool_approval_dialog_matches(state, approval_id));
        if should_clear {
            self.confirm_dialog = None;
        }
    }

    async fn interrupt_active_turn(&mut self) {
        let Some(turn_id) = self.active_turn_id.clone() else {
            self.timeline
                .push_system("no running turn to interrupt.".to_string());
            return;
        };
        let params = self.interrupt_params(turn_id);
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("interrupt")),
                method: "turn/interrupt".to_string(),
                params: Some(serde_json::to_value(params).unwrap()),
            })
            .await;
        if let Some(err) = res.error {
            self.record_error(format!("interrupt failed: {}", err.message));
        } else {
            self.push_event("interrupt requested".to_string());
        }
    }

    fn interrupt_params(&self, turn_id: String) -> TurnInterruptParams {
        TurnInterruptParams {
            thread_id: self.focused_thread_id().to_string(),
            turn_id: Some(turn_id),
        }
    }

    fn focused_thread_id(&self) -> &str {
        if let Some(thread_id) = self
            .roadmap_mode
            .as_ref()
            .and_then(|roadmap| roadmap.selected_thread_id.as_deref())
        {
            return thread_id;
        }
        self.team_ui.focused_thread_id(&self.thread_id)
    }

    fn cycle_team_focus(&mut self, next: bool) {
        self.save_focused_team_timeline();
        let changed = if next {
            self.team_ui.focus_next()
        } else {
            self.team_ui.focus_previous()
        };
        if !changed {
            return;
        }
        self.load_focused_team_timeline();
        if let Some(label) = self.team_ui.focused_label() {
            self.push_event(format!("focused {label}"));
        }
        self.timeline.focus_composer();
    }

    fn flush_streaming_animation_for_thread(&mut self, thread_id: &str) {
        if let Some(timeline) = self.team_timeline_for_thread_mut(thread_id) {
            timeline.flush_streaming_animation();
        } else {
            self.timeline.flush_streaming_animation();
        }
    }

    fn tick_streaming_animations(&mut self, now: Instant, width: u16) -> bool {
        let mut changed = self.timeline.tick_streaming_animation(now, width);
        for timeline in self.team_timelines.values_mut() {
            changed |= timeline.tick_streaming_animation(now, width);
        }
        changed
    }

    fn has_streaming_animation(&self) -> bool {
        self.timeline.has_streaming_animation()
            || self
                .team_timelines
                .values()
                .any(TimelineState::has_streaming_animation)
    }

    fn animation_poll_timeout(&self, next_tick: Instant, now: Instant) -> Duration {
        if self.has_streaming_animation() {
            stream_animation::STREAM_ANIMATION_FRAME_TIME
                .min(top_status_animation_poll_timeout(next_tick, now))
        } else {
            top_status_animation_poll_timeout(next_tick, now)
        }
    }

    fn save_focused_team_timeline(&mut self) {
        let Some(member_id) = self.team_ui.focused_member_id() else {
            return;
        };
        self.team_timelines
            .insert(member_id.to_string(), std::mem::take(&mut self.timeline));
    }

    fn load_focused_team_timeline(&mut self) {
        let Some(member_id) = self.team_ui.focused_member_id() else {
            return;
        };
        self.timeline = self
            .team_timelines
            .remove(member_id)
            .unwrap_or_else(|| TimelineState::new(self.scroll_settings, self.timeline_settings));
        self.timeline.set_settings(self.timeline_settings);
    }

    fn team_timeline_for_thread_mut(&mut self, thread_id: &str) -> Option<&mut TimelineState> {
        let member_id = self.team_ui.member_id_for_thread(thread_id)?.to_string();
        if Some(member_id.as_str()) == self.team_ui.focused_member_id() {
            return Some(&mut self.timeline);
        }
        Some(
            self.team_timelines.entry(member_id).or_insert_with(|| {
                TimelineState::new(self.scroll_settings, self.timeline_settings)
            }),
        )
    }

    async fn resolve_tool_approval(&mut self, approval_id: String, approved: bool) {
        let params = ThreadResolveApprovalParams {
            approval_id: approval_id.clone(),
            approved,
        };
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("thread/resolve_approval")),
                method: "thread/resolve_approval".to_string(),
                params: Some(serde_json::to_value(params).unwrap()),
            })
            .await;
        match decode_response::<ThreadResolveApprovalResult>(res) {
            Ok(result) if result.resolved => {}
            Ok(_) => self.record_error(format!("approval not pending: {}", short_id(&approval_id))),
            Err(err) => self.record_error(format!("thread/resolve_approval failed: {err}")),
        }
    }

    async fn refresh_thread_state(&mut self) {
        match thread_state(&self.client).await {
            Ok(state) => {
                self.policy_mode = state.mode;
                self.pending_plan_exit = state.pending_plan_exit;
            }
            Err(err) => self.record_error(format!("thread/state failed: {err}")),
        }
    }

    async fn cycle_policy_mode(&mut self) {
        let next = next_policy_mode(self.policy_mode);
        let params = ThreadSetModeParams {
            mode: next,
            reason: Some("tui mode switcher".to_string()),
        };
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("thread/set_mode")),
                method: "thread/set_mode".to_string(),
                params: Some(serde_json::to_value(params).unwrap()),
            })
            .await;
        match decode_response::<ThreadSetModeResult>(res) {
            Ok(result) => {
                self.policy_mode = result.mode;
                self.timeline.push_system(format!(
                    "policy mode set to {}.",
                    policy_mode_label(result.mode)
                ));
                self.push_event(format!(
                    "policy mode selected: {}",
                    policy_mode_label(result.mode)
                ));
            }
            Err(err) => self.record_error(format!("thread/set_mode failed: {err}")),
        }
    }

    async fn resolve_pending_plan_exit(&mut self, approved: bool) {
        let Some(pending) = self.pending_plan_exit.clone() else {
            return;
        };
        let params = ThreadExitPlanParams {
            request_id: pending.request_id.clone(),
            approved,
        };
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("thread/exit_plan")),
                method: "thread/exit_plan".to_string(),
                params: Some(serde_json::to_value(params).unwrap()),
            })
            .await;
        match decode_response::<ThreadExitPlanResult>(res) {
            Ok(result) => {
                self.policy_mode = result.mode;
                self.pending_plan_exit = None;
                self.timeline.push_system(format!(
                    "{} plan exit request {}.",
                    if approved { "approved" } else { "rejected" },
                    short_id(&pending.request_id)
                ));
            }
            Err(err) => self.record_error(format!("thread/exit_plan failed: {err}")),
        }
    }

    async fn submit_prompt(&mut self) {
        if self.active_turn_id.is_none()
            && self.image_attachments.is_empty()
            && let Some(command) = shell_command_from_input(&composer_text(&self.composer))
        {
            self.composer = composer_textarea(self.theme);
            self.slash_command_selection = 0;
            self.run_shell_command(command).await;
            return;
        }

        let Some(pending) = self.take_prepared_prompt() else {
            return;
        };

        if self.active_turn_id.is_some() {
            self.steer_prepared_prompt(pending).await;
        } else {
            self.start_prepared_prompt(pending).await;
        }
    }

    async fn handle_slash_command_key(&mut self, key: KeyEvent) -> bool {
        if key.modifiers != KeyModifiers::NONE {
            return false;
        }
        let input = composer_text(&self.composer);
        if commands::slash_query(&input).is_some() {
            self.refresh_command_catalog().await;
        }
        if self.image_attachments.is_empty() && key.code == KeyCode::Enter {
            if let Some((name, args)) = commands::command_invocation(&input, &self.command_catalog)
            {
                self.run_slash_command_invocation(name, args).await;
                return true;
            }
            if let Some((name, args)) = commands::selected_invocation(
                &input,
                &self.command_catalog,
                self.slash_command_selection,
            ) {
                self.run_slash_command_invocation(name, args).await;
                return true;
            }
        }

        let match_count = self
            .slash_command_matches()
            .map(|matches| matches.len().min(MAX_VISIBLE_SLASH_COMMANDS))
            .unwrap_or_default();
        let has_matches = match_count > 0;
        if !has_matches {
            return false;
        }
        self.slash_command_selection = self.slash_command_selection.min(match_count - 1);

        match key.code {
            KeyCode::Up => {
                self.move_slash_command_selection(-1);
                true
            }
            KeyCode::Down => {
                self.move_slash_command_selection(1);
                true
            }
            KeyCode::Tab => {
                if let Some(completed) = commands::accepted_completion(
                    &composer_text(&self.composer),
                    &self.command_catalog,
                    self.slash_command_selection,
                ) {
                    self.composer = composer_textarea(self.theme);
                    self.composer.insert_str(completed);
                }
                self.slash_command_selection = 0;
                true
            }
            _ => false,
        }
    }

    fn move_slash_command_selection(&mut self, delta: isize) {
        let count = self
            .slash_command_matches()
            .map(|matches| matches.len().min(MAX_VISIBLE_SLASH_COMMANDS))
            .unwrap_or_default();
        if count == 0 {
            self.slash_command_selection = 0;
            return;
        }
        self.slash_command_selection =
            (self.slash_command_selection as isize + delta).rem_euclid(count as isize) as usize;
    }

    async fn run_slash_command_invocation(&mut self, name: String, args: String) {
        self.composer = composer_textarea(self.theme);
        self.slash_command_selection = 0;
        match name.as_str() {
            "clear" => {
                self.timeline = TimelineState::new(self.scroll_settings, self.timeline_settings);
                self.timeline.push_system("Conversation display cleared.");
                self.push_event("slash command: /clear".to_string());
            }
            "help" => {
                self.timeline
                    .push_system(commands::help_text(&self.command_catalog));
                self.push_event("slash command: /help".to_string());
            }
            "goal" => {
                self.run_goal_slash_command(&args).await;
            }
            "retry" => {
                self.run_retry_slash_command().await;
            }
            "model" => {
                self.run_model_slash_command(&args).await;
            }
            "agents" => {
                self.run_agents_slash_command().await;
            }
            "tasks" => {
                self.run_tasks_slash_command().await;
            }
            "ps" => {
                self.run_processes_slash_command(&args).await;
            }
            "remote" => {
                self.run_remote_slash_command(&args).await;
            }
            "roadmap" => {
                self.run_roadmap_slash_command(&args);
            }
            "voice" => {
                self.run_voice_slash_command(&args).await;
            }
            "webwright" => {
                self.run_webwright_slash_command(&args).await;
            }
            _ => {
                self.run_custom_slash_command(name, args).await;
            }
        }
    }

    async fn run_custom_slash_command(&mut self, name: String, args: String) {
        let suffix = slash_command_suffix(&args);
        match self.expand_slash_command(&name, &args).await {
            Ok(expanded) => {
                let pending = PendingPrompt::with_images(
                    format!("/{name}{suffix}"),
                    expanded.message,
                    Vec::new(),
                );
                if self.active_turn_id.is_some() {
                    self.steer_prepared_prompt(pending).await;
                } else {
                    self.start_prepared_prompt(pending).await;
                }
                self.push_event(format!("slash command: /{name}{suffix}"));
            }
            Err(err) => self.record_error(format!("commands/expand failed for /{name}: {err}")),
        }
    }

    async fn refresh_command_catalog(&mut self) {
        if let Ok(commands) = thread_resume::commands_list(&self.client).await {
            self.command_catalog = commands::with_local_commands(commands);
        }
    }

    async fn run_retry_slash_command(&mut self) {
        if self.active_turn_id.is_some() {
            self.record_error(
                "retry unavailable while a turn is running; interrupt first.".to_string(),
            );
            return;
        }

        let Some(pending) = self.last_user_prompt.clone() else {
            self.record_error("nothing to retry yet.".to_string());
            return;
        };

        let cleared = self.queued_prompts.clear();
        if cleared > 0 {
            let plural = if cleared == 1 { "" } else { "s" };
            self.timeline.push_system(format!(
                "Cleared {cleared} queued follow-up input{plural} before retry."
            ));
            self.push_event(format!(
                "cleared {cleared} queued follow-up input{plural} before retry"
            ));
        }

        self.push_event("slash command: /retry".to_string());
        self.start_prepared_prompt(pending).await;
    }

    async fn run_model_slash_command(&mut self, args: &str) {
        let model = args.trim();
        if model.is_empty() {
            self.timeline.push_system(format!(
                "Active model: {}/{}. Opening model settings.",
                self.provider, self.model
            ));
            self.open_provider_popup().await;
            self.push_event("slash command: /model".to_string());
            return;
        }

        match self.providers_list().await {
            Ok(list) => {
                let selected = list.providers.iter().find_map(|provider| {
                    provider
                        .models
                        .iter()
                        .find(|candidate| candidate.id == model)
                        .map(|candidate| (provider.id.clone(), candidate.id.clone()))
                });
                if let Some((provider, model)) = selected {
                    self.select_provider_model_params(ProviderSelectParams {
                        provider,
                        model: Some(model),
                        reasoning: None,
                        thread_id: Some(self.focused_thread_id().to_string()),
                    })
                    .await;
                } else {
                    self.timeline
                        .push_error(format!("model not found for /model {model}"));
                }
            }
            Err(err) => self.record_error(format!("providers/list failed: {err}")),
        }
    }

    async fn run_agents_slash_command(&mut self) {
        match self.agents_list().await {
            Ok(result) if result.agents.is_empty() => {
                self.timeline.push_system("No configured subagents.");
                self.push_event("slash command: /agents".to_string());
            }
            Ok(result) => {
                let lines = result
                    .agents
                    .into_iter()
                    .map(|agent| {
                        let model = agent
                            .model
                            .as_deref()
                            .map(|model| format!(" [{model}]"))
                            .unwrap_or_default();
                        format!("{}{} - {}", agent.agent_type, model, agent.description)
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                self.timeline
                    .push_system(format!("Configured subagents:\n{lines}"));
                self.push_event("slash command: /agents".to_string());
            }
            Err(err) => self.record_error(format!("agents/list failed: {err}")),
        }
    }

    async fn run_tasks_slash_command(&mut self) {
        match self.tasks_list().await {
            Ok(result) if result.tasks.is_empty() => {
                self.timeline.push_system("No background tasks.");
                self.push_event("slash command: /tasks".to_string());
            }
            Ok(result) => {
                let mut lines = vec!["Background tasks:".to_string()];
                for task in result.tasks {
                    let logs = self
                        .task_get(&task.task_id)
                        .await
                        .ok()
                        .map(|result| {
                            result
                                .logs
                                .into_iter()
                                .map(|entry| entry.chunk)
                                .collect::<String>()
                        })
                        .unwrap_or_default();
                    let tail = truncate(logs.trim(), 80);
                    let tail = if tail.is_empty() {
                        String::new()
                    } else {
                        format!(" - {tail}")
                    };
                    lines.push(format!(
                        "{}\t{}\t{:?}\t{}\tcreated:{} started:{} finished:{}{}",
                        short_id(&task.task_id),
                        task.executor_id,
                        task.state,
                        task.spec.kind,
                        task.created_at.unix_timestamp(),
                        task.started_at
                            .map(|timestamp| timestamp.unix_timestamp().to_string())
                            .unwrap_or_else(|| "-".to_string()),
                        task.finished_at
                            .map(|timestamp| timestamp.unix_timestamp().to_string())
                            .unwrap_or_else(|| "-".to_string()),
                        tail
                    ));
                }
                self.timeline.push_system(lines.join("\n"));
                self.push_event("slash command: /tasks".to_string());
            }
            Err(err) => self.record_error(format!("tasks/list failed: {err}")),
        }
    }

    async fn run_remote_slash_command(&mut self, args: &str) {
        let action = args.split_whitespace().next().unwrap_or("status");
        let result = match action {
            "start" => self.remote_panel.start().await,
            "stop" => self.remote_panel.stop().await,
            "restart" | "regenerate" => self.remote_panel.start().await,
            "status" | "" => {
                if self.remote_panel.is_running() {
                    Ok(())
                } else {
                    self.remote_panel.start().await
                }
            }
            other => {
                self.timeline.push_error(format!(
                    "unknown /remote action: {other}. Use start, stop, restart, or status."
                ));
                return;
            }
        };

        match result {
            Ok(()) => {
                self.timeline.push_system(
                    render_remote_panel_lines(&self.remote_panel.snapshot()).join("\n"),
                );
                self.push_event(format!("slash command: /remote {action}"));
            }
            Err(err) => self.record_error(format!("remote {action} failed: {err}")),
        }
    }

    fn run_roadmap_slash_command(&mut self, args: &str) {
        let path = args
            .split_whitespace()
            .next()
            .filter(|value| !value.is_empty())
            .map(roadmap_slash_path);
        self.enter_roadmap_mode(path);
    }

    async fn handle_roadmap_key(&mut self, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            return false;
        }
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.roadmap_mode = None;
                self.push_event("left roadmapping mode".to_string());
                true
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_roadmap_focus(true);
                true
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_roadmap_focus(false);
                true
            }
            KeyCode::Tab => {
                if let Some(roadmap) = self.roadmap_mode.as_mut() {
                    let pane = roadmap.focus_next_pane();
                    self.push_event(format!("roadmap pane {}", pane.label()));
                }
                true
            }
            KeyCode::BackTab => {
                if let Some(roadmap) = self.roadmap_mode.as_mut() {
                    let pane = roadmap.focus_previous_pane();
                    self.push_event(format!("roadmap pane {}", pane.label()));
                }
                true
            }
            KeyCode::Char('t') => {
                if let Some(roadmap) = self.roadmap_mode.as_mut()
                    && let Some(thread_id) = roadmap.select_next_thread().map(str::to_string)
                {
                    self.push_event(format!("roadmap worker {thread_id}"));
                }
                true
            }
            KeyCode::Char('v') => {
                if let Some(roadmap) = self.roadmap_mode.as_mut() {
                    roadmap.validate_selected_document();
                    self.push_event("roadmap validated".to_string());
                }
                true
            }
            KeyCode::Char('s') => {
                let _ = self.spawn_roadmap_worker().await;
                true
            }
            KeyCode::Enter | KeyCode::Char('e') => {
                if self.roadmap_mode.as_ref().is_some_and(|roadmap| {
                    roadmap.focused_pane == crate::roadmap::RoadmapPaneFocus::Agents
                }) {
                    self.monitor_selected_roadmap_worker().await;
                } else {
                    self.execute_focused_roadmap_task().await;
                }
                true
            }
            _ => false,
        }
    }

    fn move_roadmap_focus(&mut self, forward: bool) {
        let Some(roadmap) = self.roadmap_mode.as_mut() else {
            return;
        };
        match roadmap.focused_pane {
            crate::roadmap::RoadmapPaneFocus::Plans => {
                let result = if forward {
                    roadmap.focus_next_plan()
                } else {
                    roadmap.focus_previous_plan()
                }
                .map(|plan| plan.map(str::to_string));
                match result {
                    Ok(Some(plan)) => self.push_event(format!("roadmap plan {plan}")),
                    Ok(None) => {}
                    Err(err) => self.record_error(format!("roadmap plan navigation failed: {err}")),
                }
            }
            crate::roadmap::RoadmapPaneFocus::Tasks => {
                let task_id = if forward {
                    roadmap.focus_next_task()
                } else {
                    roadmap.focus_previous_task()
                }
                .map(str::to_string);
                if let Some(task_id) = task_id {
                    self.push_event(format!("roadmap focus {task_id}"));
                }
            }
            crate::roadmap::RoadmapPaneFocus::Agents => {
                let thread_id = if forward {
                    roadmap.select_next_thread()
                } else {
                    roadmap.select_previous_thread()
                }
                .map(str::to_string);
                if let Some(thread_id) = thread_id {
                    self.push_event(format!("roadmap worker {thread_id}"));
                }
            }
            crate::roadmap::RoadmapPaneFocus::TaskDetail
            | crate::roadmap::RoadmapPaneFocus::Validation
            | crate::roadmap::RoadmapPaneFocus::Activity => {
                let label = roadmap.focused_pane.label();
                if forward {
                    roadmap.scroll_focused_pane_down();
                } else {
                    roadmap.scroll_focused_pane_up();
                }
                self.push_event(format!("roadmap {label} scroll"));
            }
        }
    }

    async fn spawn_roadmap_worker(&mut self) -> Option<ThreadAttachment> {
        let Some((path, task_id)) = self.selected_roadmap_task_ref() else {
            self.record_error("roadmap worker spawn needs a selected task".to_string());
            return None;
        };
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("roadmap/thread/spawn")),
                method: "roadmap/thread/spawn".to_string(),
                params: Some(serde_json::json!({
                    "path": path,
                    "taskId": task_id,
                })),
            })
            .await;
        match decode_response::<RoadmapThreadResponse>(res) {
            Ok(response) => {
                if let Some(roadmap) = self.roadmap_mode.as_mut() {
                    roadmap.selected_thread_id = Some(response.thread.thread_id.clone());
                    roadmap.attached_threads.push(response.thread.clone());
                }
                self.push_event(format!(
                    "spawned roadmap worker {}",
                    response.thread.thread_id
                ));
                Some(response.thread)
            }
            Err(err) => {
                self.record_error(format!("roadmap worker spawn failed: {err}"));
                None
            }
        }
    }

    async fn execute_focused_roadmap_task(&mut self) {
        let Some(roadmap) = self.roadmap_mode.as_ref() else {
            return;
        };
        let task = roadmap
            .focused_task_heading()
            .unwrap_or("focused roadmap task");
        let display = format!("Execute roadmap task: {task}");
        let message = roadmap.prompt_context(
            "Execute or continue the focused roadmap task. Use the roadmap document as source of truth, steer attached workers if present, and update task state only with evidence.",
        );
        let pending = PendingPrompt::with_images(display, message, Vec::new());
        if self.active_turn_id.is_some() {
            self.steer_prepared_prompt(pending).await;
        } else {
            self.start_prepared_prompt(pending).await;
        }
    }

    async fn monitor_selected_roadmap_worker(&mut self) {
        let Some((thread_id, task_id)) = self.roadmap_mode.as_ref().and_then(|roadmap| {
            let thread_id = roadmap.selected_thread_id.clone()?;
            let task_id = roadmap
                .attached_threads
                .iter()
                .find(|thread| thread.thread_id == thread_id)
                .and_then(|thread| thread.task_id.clone())
                .or_else(|| roadmap.focused_task_id.clone());
            Some((thread_id, task_id))
        }) else {
            self.record_error("roadmap worker monitor needs a selected worker".to_string());
            return;
        };
        match thread_resume::load_thread(&self.client, &thread_id).await {
            Ok(Some(thread)) => {
                self.roadmap_mode = None;
                self.apply_thread(thread);
                self.timeline.push_system(format!(
                    "monitoring roadmap worker {}.",
                    short_id(&thread_id)
                ));
                self.push_event(format!(
                    "monitoring roadmap worker {}",
                    short_id(&thread_id)
                ));
            }
            Ok(None) => {
                self.push_event(format!(
                    "roadmap worker {} had no thread; spawning replacement",
                    short_id(&thread_id)
                ));
                if let Some(task_id) = task_id
                    && let Some(roadmap) = self.roadmap_mode.as_mut()
                {
                    roadmap.focused_task_id = Some(task_id);
                }
                let Some(thread) = self.spawn_roadmap_worker().await else {
                    self.record_error(format!(
                        "roadmap worker thread not found: {}",
                        short_id(&thread_id)
                    ));
                    return;
                };
                match thread_resume::load_thread(&self.client, &thread.thread_id).await {
                    Ok(Some(protocol_thread)) => {
                        let replacement_id = thread.thread_id.clone();
                        self.roadmap_mode = None;
                        self.apply_thread(protocol_thread);
                        self.timeline.push_system(format!(
                            "monitoring replacement roadmap worker {}.",
                            short_id(&replacement_id)
                        ));
                        self.push_event(format!(
                            "monitoring replacement roadmap worker {}",
                            short_id(&replacement_id)
                        ));
                    }
                    Ok(None) => self.record_error(format!(
                        "replacement roadmap worker thread not found: {}",
                        short_id(&thread.thread_id)
                    )),
                    Err(err) => self
                        .record_error(format!("replacement roadmap worker monitor failed: {err}")),
                }
            }
            Err(err) => self.record_error(format!("roadmap worker monitor failed: {err}")),
        }
    }

    fn selected_roadmap_task_ref(&self) -> Option<(String, String)> {
        let roadmap = self.roadmap_mode.as_ref()?;
        Some((
            roadmap.selected_plan.clone()?,
            roadmap.focused_task_id.clone()?,
        ))
    }

    async fn expand_slash_command(
        &self,
        name: &str,
        arguments: &str,
    ) -> anyhow::Result<CommandsExpandResult> {
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("commands/expand")),
                method: "commands/expand".to_string(),
                params: Some(
                    serde_json::to_value(CommandsExpandParams {
                        name: name.to_string(),
                        arguments: arguments.to_string(),
                        workspace: None,
                    })
                    .unwrap(),
                ),
            })
            .await;
        decode_response(res)
    }

    fn slash_command_matches(&self) -> Option<Vec<&CommandDescriptor>> {
        if !self.image_attachments.is_empty() {
            return None;
        }
        let input = composer_text(&self.composer);
        if !commands::should_show_menu(&input) {
            return None;
        }
        Some(commands::matching_commands(&self.command_catalog, &input))
    }

    fn take_prepared_prompt(&mut self) -> Option<PendingPrompt> {
        let text = composer_text(&self.composer).trim().to_string();
        if text.is_empty() && self.image_attachments.is_empty() {
            return None;
        }
        let images = match image_inputs_from_attachments(&self.image_attachments) {
            Ok(images) => images,
            Err(err) => {
                self.timeline
                    .push_error(format!("image attachment failed: {err}"));
                return None;
            }
        };
        let attachments = std::mem::take(&mut self.image_attachments);
        self.composer = composer_textarea(self.theme);
        self.slash_command_selection = 0;
        let display = transcript_message_with_image_attachments(&text, &attachments);
        let message = self
            .roadmap_mode
            .as_ref()
            .map(|roadmap| roadmap.prompt_context(&text))
            .unwrap_or(text);
        Some(PendingPrompt::with_images(display, message, images))
    }

    fn has_prepared_prompt(&self) -> bool {
        !composer_text(&self.composer).trim().is_empty() || !self.image_attachments.is_empty()
    }

    async fn start_prepared_prompt(&mut self, pending: PendingPrompt) {
        self.last_user_prompt = Some(pending.clone());
        self.timeline.push_user(pending.display.clone());
        self.thread_message_count = self.thread_message_count.saturating_add(1);
        if self.thread_title.is_none() {
            self.thread_title = Some(truncate(&pending.display, 72));
        }
        let thread_id = self.focused_thread_id().to_string();
        let params = TurnStartParams {
            thread_id,
            input: pending_turn_input(pending.message, pending.images),
            prompt: None,
            task_ledger_required: false,
        };
        let client = self.client.clone();
        tokio::spawn(async move {
            let _ = client
                .send_request(JsonRpcRequest {
                    jsonrpc: "2.0".to_string(),
                    id: Some(serde_json::json!("turn/start")),
                    method: "turn/start".to_string(),
                    params: Some(serde_json::to_value(params).unwrap()),
                })
                .await;
        });
    }

    async fn steer_prepared_prompt(&mut self, pending: PendingPrompt) {
        let Some(turn_id) = self.active_turn_id.clone() else {
            self.queue_prepared_prompt(pending);
            return;
        };
        self.timeline
            .push_user(format!("steer: {}", pending.display));
        self.push_event("steer queued for active turn".to_string());
        let thread_id = self.focused_thread_id().to_string();
        let params = TurnSteerParams {
            thread_id,
            expected_turn_id: turn_id,
            input: pending_turn_input(pending.message, pending.images),
            prompt: None,
        };
        let client = self.client.clone();
        tokio::spawn(async move {
            let _ = client
                .send_request(JsonRpcRequest {
                    jsonrpc: "2.0".to_string(),
                    id: Some(serde_json::json!("turn/steer")),
                    method: "turn/steer".to_string(),
                    params: Some(serde_json::to_value(params).unwrap()),
                })
                .await;
        });
    }

    fn queue_current_prompt(&mut self) -> bool {
        let Some(pending) = self.take_prepared_prompt() else {
            return false;
        };
        self.queue_prepared_prompt(pending);
        true
    }

    fn queue_prepared_prompt(&mut self, pending: PendingPrompt) {
        self.queued_prompts.push(pending);
        self.push_event(queue_status(self.queued_prompts.len()));
    }

    fn can_edit_queued_prompt(&self) -> bool {
        self.timeline.focus() == TimelineFocus::Composer
            && composer_text(&self.composer).trim().is_empty()
            && !self.queued_prompts.is_empty()
    }

    fn edit_latest_queued_prompt(&mut self) -> bool {
        let Some(pending) = self.queued_prompts.pop_back() else {
            return false;
        };
        if !pending.images.is_empty() {
            self.queued_prompts.push(pending);
            self.record_error("queued image prompts cannot be edited in place yet.".to_string());
            return false;
        }
        self.composer = composer_textarea(self.theme);
        self.composer.insert_str(pending.message);
        self.slash_command_selection = 0;
        self.timeline.focus_composer();
        self.push_event(queue_status(self.queued_prompts.len()));
        true
    }

    async fn submit_next_queued_prompt(&mut self) {
        if let Some(next) = self.queued_prompts.pop_front() {
            self.start_prepared_prompt(next).await;
        }
    }

    fn handle_paste(&mut self, text: String) {
        if self.tool_detail_modal.is_some() {
            return;
        }
        if self.confirm_dialog.is_some() {
            return;
        }
        if self.show_provider_popup {
            self.provider_menu_filter
                .push_str(&text.replace(['\r', '\n'], " "));
            if self.provider_popup_screen != ProviderPopupScreen::ApiKey {
                self.clamp_provider_menu_selection();
            }
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
        self.slash_command_selection = 0;
    }

    fn handle_mouse(&mut self, mouse: MouseEvent) {
        self.update_context_counter_hover(&mouse);
        if let Some(modal) = self.tool_detail_modal.as_mut() {
            modal.handle_mouse(mouse);
            return;
        }
        if self.confirm_dialog.is_some() || self.show_provider_popup {
            return;
        }
        if self.handle_plan_counter_mouse(mouse) {
            return;
        }
        // Give an in-progress text selection first chance to handle drag/up,
        // but let fresh clicks hit interactive timeline rows before the
        // transcript selection layer. Otherwise selectable transcript text
        // swallows tool-row clicks and prevents the tool detail modal from
        // opening.
        if self.mouse_selection.is_some() && self.handle_mouse_selection(mouse) {
            return;
        }
        if self.timeline.handle_mouse(mouse) {
            if let Some(detail) = self.timeline.take_requested_detail() {
                self.tool_detail_modal = Some(ToolDetailModal::new(detail, self.scroll_settings));
                self.push_event("tool detail opened".to_string());
                return;
            }
            self.push_event("timeline selected".to_string());
            return;
        }
        let _ = self.handle_mouse_selection(mouse);
    }

    fn handle_plan_counter_mouse(&mut self, mouse: MouseEvent) -> bool {
        if !matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
            return false;
        }
        let Some(area) = self.last_plan_counter_area else {
            return false;
        };
        if !rect_contains(area, mouse.column, mouse.row) {
            return false;
        }
        self.toggle_plan_panel();
        true
    }

    fn handle_mouse_selection(&mut self, mouse: MouseEvent) -> bool {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let Some(point) = self.selection_point(mouse) else {
                    return false;
                };
                self.mouse_selection = Some(MouseSelection {
                    anchor: point,
                    cursor: point,
                    dragging: false,
                });
                true
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some(point) = self.selection_point(mouse)
                    && let Some(selection) = self.mouse_selection.as_mut()
                {
                    selection.cursor = point;
                    selection.dragging = true;
                    return true;
                }
                self.mouse_selection.is_some()
            }
            MouseEventKind::Up(MouseButton::Left) => {
                let Some(mut selection) = self.mouse_selection.take() else {
                    return false;
                };
                if let Some(point) = self.selection_point(mouse) {
                    selection.cursor = point;
                }
                if !selection.dragging {
                    return true;
                }
                let Some(text) = selected_text(&self.selectable_lines, &selection) else {
                    return true;
                };
                tokio::spawn(async move {
                    let _ = copy_selection_to_clipboards(text).await;
                });
                self.copied_helper = Some(CopiedHelper {
                    shown_at: Instant::now(),
                });
                self.push_event("selection copied".to_string());
                true
            }
            _ => false,
        }
    }

    fn selection_point(&self, mouse: MouseEvent) -> Option<SelectionPoint> {
        self.selectable_lines
            .iter()
            .find(|line| line.row == mouse.row)
            .map(|line| SelectionPoint {
                row: mouse.row,
                column: mouse.column.min(line.text.chars().count() as u16),
            })
    }

    fn update_context_counter_hover(&mut self, mouse: &MouseEvent) {
        if !matches!(
            mouse.kind,
            MouseEventKind::Moved
                | MouseEventKind::Down(_)
                | MouseEventKind::Up(_)
                | MouseEventKind::Drag(_)
                | MouseEventKind::ScrollDown
                | MouseEventKind::ScrollUp
                | MouseEventKind::ScrollLeft
                | MouseEventKind::ScrollRight
        ) {
            return;
        }
        self.context_counter_hovered = self.context_window_counter().is_some_and(|counter| {
            counter.hit_test(self.last_frame_width, mouse.row, mouse.column)
        });
    }

    async fn run_shell_command(&mut self, command: String) {
        self.timeline.push_shell(command.clone());
        self.push_event(format!("shell command started: {command}"));
        match run_shell_command(command.clone()).await {
            Ok(output) => {
                self.timeline.push_shell_output(output);
                self.push_event(format!("shell command finished: {command}"));
            }
            Err(err) => {
                self.record_error(format!("shell command failed: {err}"));
            }
        }
    }

    fn render(&mut self, f: &mut Frame<'_>) {
        let area = f.area();
        self.last_frame_width = area.width;
        if let Some(bg) = self.theme.body_background {
            // Theme opted out of transparency — paint the whole frame so
            // subsequent widgets (which mostly use Style::default()) sit on
            // the themed surface instead of the terminal's native background.
            f.render_widget(Block::default().style(Style::default().bg(bg)), area);
        }
        if let Some(roadmap) = self.roadmap_mode.as_ref() {
            let activity = self
                .timeline
                .render_with_frame(self.theme, area, self.animation_frame)
                .text;
            render_roadmap_workspace(
                f,
                area,
                roadmap,
                self.theme,
                RoadmapWorkspaceMeta {
                    model: self.model.clone(),
                    status: if self.active_turn_id.is_some() {
                        working_status_label(self.compaction_active).to_string()
                    } else {
                        "roadmap ready".to_string()
                    },
                    active_turn: self.active_turn_id.is_some(),
                },
                activity,
            );
            self.render_overlays(f, area);
            return;
        }
        style_composer_for_current_mode(&mut self.composer, self.theme, self.policy_mode);
        let event_height = event_log_height(self.show_event_log, self.events.len());
        let attachment_height = image_attachment_height(self.image_attachments.len());
        let queue_height = queued_prompt_height(self.queued_prompts.len());
        let plan_height = plan_panel_height(&self.plan_panel);
        let slash_matches = self
            .slash_command_matches()
            .map(|matches| matches.into_iter().cloned().collect::<Vec<_>>());
        let slash_height = slash_command_menu_height(slash_matches.as_deref());
        let slash_preview_height = slash_command_preview_height(slash_matches.as_deref());
        let composer_height = self.composer.measure(area.width).preferred_rows;
        let mut constraints = top_layout_constraints().to_vec();
        if event_height > 0 {
            constraints.push(Constraint::Length(event_height));
        }
        if attachment_height > 0 {
            constraints.push(Constraint::Length(attachment_height));
        }
        if queue_height > 0 {
            constraints.push(Constraint::Length(queue_height));
        }
        if self.active_turn_id.is_some() {
            constraints.push(Constraint::Length(1));
        }
        if plan_height > 0 {
            constraints.push(Constraint::Length(plan_height));
        }
        if slash_preview_height > 0 {
            constraints.push(Constraint::Length(slash_preview_height));
        }
        constraints.push(Constraint::Length(composer_height));
        if slash_height > 0 {
            constraints.push(Constraint::Length(slash_height));
        }
        constraints.push(Constraint::Length(1));

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(area);

        let transcript_index = 1;
        f.render_widget(self.header(area.width), chunks[0]);
        if self.roadmap_mode.is_some() {
            f.render_widget(
                self.roadmap_document(chunks[transcript_index]),
                chunks[transcript_index],
            );
        } else {
            f.render_widget(
                self.transcript(chunks[transcript_index]),
                chunks[transcript_index],
            );
        }

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
        if queue_height > 0 {
            f.render_widget(self.queued_prompt_bar(), chunks[composer_index]);
            composer_index += 1;
        }
        if self.active_turn_id.is_some() {
            f.render_widget(self.working_line(), chunks[composer_index]);
            composer_index += 1;
        }
        if plan_height > 0 {
            f.render_widget(
                render_plan_panel(&self.plan_panel, self.theme),
                chunks[composer_index],
            );
            composer_index += 1;
        }
        if slash_preview_height > 0 {
            f.render_widget(
                self.slash_command_preview(slash_matches.as_deref()),
                chunks[composer_index],
            );
            composer_index += 1;
        }
        self.render_copied_helper(f, chunks[composer_index], Instant::now());
        f.render_widget(&self.composer, chunks[composer_index]);
        self.render_voice_transcribing_helper(f, chunks[composer_index]);
        self.render_plan_counter(f, chunks[composer_index]);
        composer_index += 1;
        if slash_height > 0 {
            f.render_widget(
                self.slash_command_menu(slash_matches.as_deref()),
                chunks[composer_index],
            );
            composer_index += 1;
        }
        f.render_widget(self.footer(area.width), chunks[composer_index]);

        self.render_overlays(f, area);
    }

    fn render_overlays(&mut self, f: &mut Frame<'_>, area: Rect) {
        if self.show_provider_popup {
            self.render_provider_popup(f, area);
        }
        if self.plugin_browser.is_some() {
            self.render_plugin_browser(f, area);
        }
        if let Some(dialog) = self.confirm_dialog.clone() {
            self.render_confirm_dialog(f, area, dialog);
        }
        if self.show_shortcuts_dialog {
            shortcuts::render_shortcuts_dialog(f, area, self.theme);
        }
        if let Some(modal) = &self.tool_detail_modal {
            render_tool_detail_modal(f, area, modal, self.theme);
        }
    }

    fn render_copied_helper(&mut self, f: &mut Frame<'_>, composer_area: Rect, now: Instant) {
        if !self.copied_helper.is_some_and(|helper| helper.visible(now)) {
            self.copied_helper = None;
            return;
        }

        let Some(area) = copied_helper_area(composer_area) else {
            return;
        };
        f.render_widget(copied_helper_widget(self.theme), area);
    }

    fn render_voice_transcribing_helper(&self, f: &mut Frame<'_>, composer_area: Rect) {
        if !self.voice.is_transcribing() {
            return;
        }
        let Some(area) = voice_helper_area(composer_area) else {
            return;
        };
        f.render_widget(Clear, area);
        f.render_widget(
            voice_transcribing_widget(self.theme, self.working_spinner, self.animation_frame),
            area,
        );
    }

    fn render_plan_counter(&mut self, f: &mut Frame<'_>, composer_area: Rect) {
        let Some(area) = plan_counter_area(composer_area, &self.plan_panel) else {
            self.last_plan_counter_area = None;
            return;
        };
        self.last_plan_counter_area = Some(area);
        f.render_widget(render_plan_counter(&self.plan_panel, self.theme), area);
    }

    fn working_line(&self) -> Paragraph<'static> {
        let elapsed = self.active_turn_timer.elapsed(Instant::now());
        let status = self
            .timeline
            .latest_reasoning_heading()
            .unwrap_or_else(|| working_status_label(self.compaction_active).to_string());
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!(
                    " {} ",
                    padded_spinner_frame(self.working_spinner, self.animation_frame)
                ),
                self.theme.running(),
            ),
            Span::styled(
                format!(
                    "{status} ({} - esc to interrupt)",
                    format_working_elapsed(elapsed)
                ),
                self.theme.muted(),
            ),
        ]))
    }

    fn header(&self, width: u16) -> Paragraph<'static> {
        let model_label = format!("{}/{}", self.provider, self.model);
        let left = vec![
            Span::styled(" roder", self.theme.accent()),
            Span::styled(format!("  {model_label}"), self.theme.text()),
            Span::styled(
                format!("  reasoning {}", self.reasoning_effort),
                self.theme.muted(),
            ),
            Span::styled(
                format!("  thread {}", short_id(&self.thread_id)),
                self.theme.muted(),
            ),
        ];
        let mut left = left;
        if let Some(label) = self.team_ui.focused_label() {
            left.push(Span::styled(format!("  {label}"), self.theme.accent_soft()));
        }
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
        let mut right = vec![Span::styled(turn.to_string(), right_style)];
        if let Some(counter) = self.context_window_counter() {
            right.push(Span::styled(" ".to_string(), self.theme.text()));
            right.extend(counter.spans(self.theme));
        }
        Paragraph::new(line_with_gap(left, right, width, self.theme.text()))
    }

    fn context_window_counter(&self) -> Option<ContextWindowCounter> {
        let max_tokens = u64::from(self.model_context_window?);
        (max_tokens > 0).then_some(ContextWindowCounter {
            used_tokens: self.context_window_tokens,
            max_tokens,
            hovered: self.context_counter_hovered,
        })
    }

    fn transcript(&mut self, area: Rect) -> Paragraph<'static> {
        let render = self
            .timeline
            .render_with_frame(self.theme, area, self.animation_frame);
        self.selectable_lines = selectable_lines_from_text(&render.text, area, render.text_scroll);
        let text = if let Some(selection) = &self.mouse_selection {
            if selection.dragging {
                highlight_selection(render.text, area, render.text_scroll, selection, self.theme)
            } else {
                render.text
            }
        } else {
            render.text
        };
        Paragraph::new(text)
            .style(self.theme.text())
            .scroll((render.text_scroll, 0))
            .wrap(Wrap { trim: false })
    }

    fn roadmap_document(&mut self, area: Rect) -> Paragraph<'static> {
        self.selectable_lines = Vec::new();
        let mut text = self
            .roadmap_mode
            .as_ref()
            .map(RoadmapModeState::render_text)
            .unwrap_or_else(|| Text::from("Roadmap mode"));
        let activity = self
            .timeline
            .render_with_frame(self.theme, area, self.animation_frame);
        if !activity.text.lines.is_empty() {
            text.lines.push(Line::from(""));
            text.lines.push(Line::from("Activity Evidence"));
            text.lines.extend(activity.text.lines.into_iter().take(6));
        }
        Paragraph::new(text)
            .style(self.theme.text())
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

    fn queued_prompt_bar(&self) -> Paragraph<'static> {
        let hidden = self.queued_prompts.len().saturating_sub(3);
        let mut lines = Vec::new();
        lines.push(Line::from(Span::styled(
            "Queued follow-up inputs",
            self.theme.strong(),
        )));
        lines.extend(self.queued_prompts.displays().take(3).map(|display| {
            Line::from(vec![
                Span::styled("↳ ", self.theme.subtle()),
                Span::styled(truncate(display, 96), self.theme.muted()),
            ])
        }));
        if hidden > 0 {
            lines.push(Line::from(Span::styled(
                format!("↳ ... {hidden} more queued input{}", plural_s(hidden)),
                self.theme.muted(),
            )));
        }
        Paragraph::new(Text::from(lines))
            .style(self.theme.text())
            .wrap(Wrap { trim: false })
    }

    fn slash_command_menu(&self, matches: Option<&[CommandDescriptor]>) -> Paragraph<'static> {
        let Some(matches) = matches else {
            return Paragraph::new(Text::default());
        };
        let selected_index = self.slash_command_selection.min(
            matches
                .len()
                .min(MAX_VISIBLE_SLASH_COMMANDS)
                .saturating_sub(1),
        );
        let visible = matches
            .iter()
            .take(MAX_VISIBLE_SLASH_COMMANDS)
            .enumerate()
            .map(|(index, command)| {
                let selected = index == selected_index;
                let marker = if selected { ">" } else { " " };
                let style = if selected {
                    self.theme.selected()
                } else {
                    self.theme.text()
                };
                let mut spans = vec![
                    Span::styled(format!(" {marker} "), self.theme.subtle()),
                    Span::styled(format!("/{}", command.name), style),
                ];
                if let Some(hint) = &command.argument_hint {
                    spans.push(Span::styled(format!(" {hint}"), self.theme.subtle()));
                }
                if let Some(description) = &command.description {
                    spans.push(Span::styled(
                        format!(" - {}", truncate(description, 72)),
                        self.theme.muted(),
                    ));
                }
                if let Some(warning) = commands::command_warning(command) {
                    spans.push(Span::styled(format!("  {warning}"), self.theme.shell()));
                }
                Line::from(spans)
            });
        let mut lines = Vec::new();
        lines.push(Line::from(vec![
            Span::styled(" Slash commands", self.theme.strong()),
            Span::styled(
                "  tab complete  enter run  up/down select",
                self.theme.subtle(),
            ),
        ]));
        lines.extend(visible);

        Paragraph::new(Text::from(lines)).style(self.theme.text())
    }

    fn slash_command_preview(&self, matches: Option<&[CommandDescriptor]>) -> Paragraph<'static> {
        let command = matches.and_then(|matches| {
            matches.get(
                self.slash_command_selection
                    .min(matches.len().saturating_sub(1)),
            )
        });
        let Some(command) = command else {
            return Paragraph::new(Text::default());
        };
        let mut spans = vec![
            Span::styled(" Preview collapsed ", self.theme.subtle()),
            Span::styled(format!("/{}", command.name), self.theme.accent()),
        ];
        if let Some(description) = command.description.as_deref() {
            spans.push(Span::styled(
                format!(" -> {}", truncate(description, 96)),
                self.theme.muted(),
            ));
        }
        if let Some(warning) = commands::command_warning(command) {
            spans.push(Span::styled(format!("  {warning}"), self.theme.shell()));
        }
        Paragraph::new(Line::from(spans)).style(self.theme.text())
    }

    fn footer(&self, width: u16) -> Paragraph<'static> {
        let status = if self.active_turn_id.is_some() {
            "running"
        } else if !self.queued_prompts.is_empty() {
            "queued"
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
        let voice_hint = self
            .voice
            .footer_hint(composer_text(&self.composer).trim().is_empty())
            .unwrap_or_default();
        let roadmap_hint = self
            .roadmap_mode
            .as_ref()
            .map(|state| format!("  roadmap:{}", state.label()))
            .unwrap_or_default();
        let goal_hint = self
            .current_goal
            .as_ref()
            .map(|goal| format!("  goal:{}", goals::goal_footer_label(goal)))
            .unwrap_or_default();
        let shortcut_context = match self.timeline.focus() {
            TimelineFocus::Timeline => FooterShortcutContext::Timeline,
            TimelineFocus::Composer if self.active_turn_id.is_some() => {
                FooterShortcutContext::ComposerRunning
            }
            TimelineFocus::Composer => FooterShortcutContext::ComposerIdle,
        };
        let interaction_hint =
            shortcuts::footer_hint(shortcut_context, !self.plan_panel.is_empty());
        Paragraph::new(line_with_gap(
            vec![Span::styled(
                format!(
                    " {status}{queue_hint}{pending_hint}{shell_hint}{voice_hint}{roadmap_hint}{goal_hint}  {interaction_hint}",
                    queue_hint = if self.queued_prompts.is_empty() {
                        String::new()
                    } else {
                        format!("  {}", queue_status(self.queued_prompts.len()))
                    },
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
        let items: Vec<ListItem> = if self.provider_popup_screen == ProviderPopupScreen::ApiKey {
            provider_api_key_items(
                self.pending_api_key_provider.as_ref(),
                &self.provider_menu_filter,
                self.theme,
            )
        } else if visible_items.is_empty() {
            vec![ListItem::new(Line::from(Span::styled(
                "No matches",
                self.theme.muted(),
            )))]
        } else {
            visible_items
                .iter()
                .map(|item| {
                    let marker = match item {
                        ProviderMenuItem::Section(_) => "  ",
                        ProviderMenuItem::Provider(provider) if provider.authenticated => "✓ ",
                        ProviderMenuItem::DefaultMode(mode) if *mode == self.policy_mode => "✓ ",
                        ProviderMenuItem::Spinner(spinner) if *spinner == self.working_spinner => {
                            "✓ "
                        }
                        ProviderMenuItem::WebSearchMode(mode) if *mode == self.web_search_mode => {
                            "✓ "
                        }
                        ProviderMenuItem::VoiceModel(choice)
                            if self.voice.provider() == Some(choice.provider_id.as_str())
                                && self.voice.model() == Some(choice.model_id.as_str()) =>
                        {
                            "✓ "
                        }
                        ProviderMenuItem::ShellChoice(shell) if shell == &self.command_shell => {
                            "✓ "
                        }
                        ProviderMenuItem::FileBackedDynamicContextToggle(true) => "✓ ",
                        ProviderMenuItem::MessageFoldingToggle(true) => "✓ ",
                        ProviderMenuItem::Thread(thread) if thread.id == self.thread_id => "✓ ",
                        ProviderMenuItem::Theme(id)
                            if self.active_theme_id.as_deref() == Some(id.as_str()) =>
                        {
                            "✓ "
                        }
                        ProviderMenuItem::Models
                        | ProviderMenuItem::Providers
                        | ProviderMenuItem::Settings
                        | ProviderMenuItem::RunnerSettings
                        | ProviderMenuItem::SpinnerSettings
                        | ProviderMenuItem::WebSearchSettings
                        | ProviderMenuItem::VoiceModelSettings
                        | ProviderMenuItem::ShellSettings(_)
                        | ProviderMenuItem::ThemesSettings
                        | ProviderMenuItem::MarketplacesSettings
                        | ProviderMenuItem::PluginBrowser
                        | ProviderMenuItem::ResumeThreads
                        | ProviderMenuItem::Reasoning(_) => "› ",
                        ProviderMenuItem::Back => "‹ ",
                        _ => "• ",
                    };
                    let label_style = if matches!(item, ProviderMenuItem::Section(_)) {
                        self.theme.accent()
                    } else {
                        self.theme.text()
                    };
                    ListItem::new(Line::from(vec![
                        Span::styled(marker, self.theme.subtle()),
                        Span::styled(item.label(), label_style),
                    ]))
                })
                .collect()
        };
        let title = match self.provider_popup_screen {
            ProviderPopupScreen::Main => " Menu (Enter select, Esc close) ",
            ProviderPopupScreen::Providers => " Providers (Enter select, Esc back) ",
            ProviderPopupScreen::ApiKey => " Paste API key (Enter save, Esc back) ",
            ProviderPopupScreen::Models => " Models by provider (Enter select, Esc back) ",
            ProviderPopupScreen::Reasoning => " Reasoning effort (Enter select, Esc back) ",
            ProviderPopupScreen::Settings => " Settings (Enter select, Esc back) ",
            ProviderPopupScreen::Runners => " Runners (Enter select, Esc back) ",
            ProviderPopupScreen::Spinner => " Working spinner (Enter select, Esc back) ",
            ProviderPopupScreen::WebSearch => " Web search provider (Enter select, Esc back) ",
            ProviderPopupScreen::VoiceModels => " Voice model (Enter select, Esc back) ",
            ProviderPopupScreen::Shell => " Shell command shell (Enter select, Esc back) ",
            ProviderPopupScreen::Resume => " Resume thread (Enter select, Esc back) ",
            ProviderPopupScreen::Themes => " Themes (Enter select, Esc back) ",
            ProviderPopupScreen::Marketplaces => " Plugin marketplaces (Enter select, Esc back) ",
        };
        let borders = if self.theme.borders_visible {
            Borders::ALL
        } else {
            Borders::NONE
        };
        let block = Block::default()
            .borders(borders)
            .border_type(self.theme.border_type)
            .style(self.theme.dialog_surface())
            .border_style(self.theme.dialog())
            .title(Span::styled(title, self.theme.accent()));
        let inner = block.inner(menu_area);
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1)])
            .split(inner);
        let menu = List::new(items)
            .style(self.theme.dialog_surface())
            .highlight_style(self.theme.selected())
            .highlight_symbol("› ");
        f.render_widget(Clear, menu_area);
        f.render_widget(block, menu_area);
        f.render_widget(
            Paragraph::new(
                if self.provider_popup_screen == ProviderPopupScreen::ApiKey {
                    provider_api_key_input_line(&self.provider_menu_filter, self.theme)
                } else {
                    provider_search_line(&self.provider_menu_filter, self.theme)
                },
            )
            .style(self.theme.dialog_surface()),
            chunks[0],
        );
        f.render_stateful_widget(menu, chunks[1], &mut self.provider_state);
    }

    fn render_confirm_dialog(&self, f: &mut Frame<'_>, area: Rect, dialog: ConfirmDialogState) {
        dialog::render_confirm_dialog(f, area, dialog, self.theme);
    }

    async fn open_provider_popup(&mut self) {
        match self.providers_list().await {
            Ok(list) => {
                self.provider = list.active_provider.clone();
                self.model = list.active_model.clone();
                self.reasoning_effort = list.active_reasoning.clone();
                self.provider_choices = provider_choices_from_list(&list);
                self.model_options = provider_options_from_list(&list);
                self.model_context_window =
                    context_window_from_options(&self.model_options, &self.provider, &self.model)
                        .or_else(|| context_window_for_model(&self.model));
                self.pending_reasoning_model = None;
                self.pending_api_key_provider = None;
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

    async fn refresh_providers_list(&mut self) -> anyhow::Result<()> {
        let list = self.providers_list().await?;
        self.provider = list.active_provider.clone();
        self.model = list.active_model.clone();
        self.reasoning_effort = list.active_reasoning.clone();
        self.provider_choices = provider_choices_from_list(&list);
        self.model_options = provider_options_from_list(&list);
        self.model_context_window =
            context_window_from_options(&self.model_options, &self.provider, &self.model)
                .or_else(|| context_window_for_model(&self.model));
        Ok(())
    }

    async fn clear_or_logout_provider(&mut self, provider: ProviderChoice) {
        if provider.auth_type == ProviderAuthType::OAuth {
            if let Some(auth_flow) = ProviderAuthFlow::for_provider(&provider.provider_id) {
                let res = self
                    .client
                    .send_request(JsonRpcRequest {
                        jsonrpc: "2.0".to_string(),
                        id: Some(serde_json::json!(auth_flow.logout_method)),
                        method: auth_flow.logout_method.to_string(),
                        params: None,
                    })
                    .await;
                match decode_response::<serde_json::Value>(res) {
                    Ok(_) => {
                        self.timeline
                            .push_system(format!("Logged out of {}.", auth_flow.display_name));
                        self.push_event(format!("logged out of: {}", provider.provider_id));
                    }
                    Err(err) => {
                        self.record_error(format!("Logout failed: {err}"));
                    }
                }
            } else {
                self.record_error(format!(
                    "provider {} requires OAuth; no logout flow is available",
                    provider.provider_id
                ));
            }
        } else if provider.auth_type == ProviderAuthType::ApiKey {
            let res = self
                .client
                .send_request(JsonRpcRequest {
                    jsonrpc: "2.0".to_string(),
                    id: Some(serde_json::json!("providers/clear")),
                    method: "providers/clear".to_string(),
                    params: Some(
                        serde_json::to_value(ProviderClearParams {
                            provider: provider.provider_id.clone(),
                        })
                        .unwrap(),
                    ),
                })
                .await;
            match decode_response::<ProviderClearResult>(res) {
                Ok(_) => {
                    self.timeline
                        .push_system(format!("Cleared API key for {}.", provider.name));
                    self.push_event(format!("provider cleared: {}", provider.provider_id));
                }
                Err(err) => {
                    self.record_error(format!("providers/clear failed: {err}"));
                }
            }
        }

        // Refresh the provider list
        if let Err(err) = self.refresh_providers_list().await {
            self.record_error(format!("Failed to refresh providers: {err}"));
            self.show_provider_popup = false;
            return;
        }

        // Recreate the providers submenu
        let saved_selected = self.provider_state.selected();
        self.provider_menu_items = providers_menu_items(&self.provider_choices);
        if let Some(sel) = saved_selected {
            if sel < self.provider_menu_items.len() {
                self.provider_state.select(Some(sel));
            } else if !self.provider_menu_items.is_empty() {
                self.provider_state
                    .select(Some(self.provider_menu_items.len() - 1));
            } else {
                self.provider_state.select(None);
            }
        }
    }

    async fn speech_providers_list(&self) -> anyhow::Result<SpeechProvidersListResult> {
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("speech/providers/list")),
                method: "speech/providers/list".to_string(),
                params: None,
            })
            .await;
        decode_response(res)
    }

    async fn agents_list(&self) -> anyhow::Result<AgentsListResult> {
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("agents/list")),
                method: "agents/list".to_string(),
                params: None,
            })
            .await;
        decode_response(res)
    }

    async fn tasks_list(&self) -> anyhow::Result<TasksListResult> {
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("tasks/list")),
                method: "tasks/list".to_string(),
                params: None,
            })
            .await;
        decode_response(res)
    }

    async fn runners_list(&self) -> anyhow::Result<RunnersListResult> {
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("runners/list")),
                method: "runners/list".to_string(),
                params: None,
            })
            .await;
        decode_response(res)
    }

    async fn select_runner(&mut self, destination_id: String, provider_id: String) {
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("runners/select")),
                method: "runners/select".to_string(),
                params: Some(
                    serde_json::to_value(RunnersSelectParams {
                        destination_id: destination_id.clone(),
                        provider_id: Some(provider_id.clone()),
                        config: serde_json::Value::Null,
                        manifest: roder_api::remote_runner::RunnerManifest::default(),
                    })
                    .unwrap(),
                ),
            })
            .await;
        match decode_response::<RunnersSelectResult>(res) {
            Ok(result) => {
                let label = runner::runner_status_label(result.active.as_ref());
                self.timeline.push_system(label);
                self.push_event(format!(
                    "runner selected: {destination_id} via {provider_id}"
                ));
            }
            Err(err) => self.record_error(format!("runners/select failed: {err}")),
        }
    }

    async fn task_get(&self, task_id: &str) -> anyhow::Result<TasksGetResult> {
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("tasks/get")),
                method: "tasks/get".to_string(),
                params: Some(serde_json::to_value(TasksGetParams {
                    task_id: task_id.to_string(),
                })?),
            })
            .await;
        decode_response(res)
    }

    async fn set_web_search_mode(&mut self, mode: HostedWebSearchMode) {
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("settings/set_web_search")),
                method: "settings/set_web_search".to_string(),
                params: Some(serde_json::to_value(SettingsSetWebSearchParams { mode }).unwrap()),
            })
            .await;

        match decode_response::<SettingsSetWebSearchResult>(res) {
            Ok(result) => {
                self.web_search_mode = result.web_search.mode;
                let label = web_search_mode_label(result.web_search.mode);
                self.timeline
                    .push_system(format!("web search provider set to {label}."));
                self.push_event(format!("web search provider selected: {label}"));
                self.show_provider_popup = false;
            }
            Err(err) => {
                self.record_error(format!("failed to set web search provider: {err}"));
                self.show_provider_popup = false;
            }
        }
    }

    async fn set_search_index_enabled(&mut self, enabled: bool) {
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("settings/set_search_index")),
                method: "settings/set_search_index".to_string(),
                params: Some(
                    serde_json::to_value(SettingsSetSearchIndexParams { enabled }).unwrap(),
                ),
            })
            .await;

        match decode_response::<SettingsSetSearchIndexResult>(res) {
            Ok(result) => {
                self.search_index_enabled = result.search_index.enabled;
                self.provider_menu_items = settings_menu_items(
                    self.timeline_settings,
                    self.search_index_enabled,
                    &self.command_shell,
                    self.file_backed_dynamic_context,
                );
                self.timeline.push_system(format!(
                    "instant regex search {}.",
                    if self.search_index_enabled {
                        "enabled"
                    } else {
                        "disabled"
                    }
                ));
                self.push_event(format!(
                    "instant regex search: {}",
                    if self.search_index_enabled {
                        "enabled"
                    } else {
                        "disabled"
                    }
                ));
            }
            Err(err) => {
                self.record_error(format!("failed to set instant regex search: {err}"));
            }
        }
    }

    async fn set_command_shell(&mut self, shell: String) {
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("settings/set_shell")),
                method: "settings/set_shell".to_string(),
                params: Some(serde_json::to_value(SettingsSetShellParams { shell }).unwrap()),
            })
            .await;

        match decode_response::<SettingsSetShellResult>(res) {
            Ok(result) => {
                self.command_shell = result.shell.shell;
                self.command_shell_options = result.shell.options;
                self.timeline.push_system(format!(
                    "shell command shell set to {}.",
                    self.command_shell
                ));
                self.push_event(format!(
                    "shell command shell selected: {}",
                    self.command_shell
                ));
                self.show_provider_popup = false;
            }
            Err(err) => {
                self.record_error(format!("failed to set shell command shell: {err}"));
                self.show_provider_popup = false;
            }
        }
    }

    async fn set_default_mode(&mut self, mode: PolicyMode) {
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("settings/set_default_mode")),
                method: "settings/set_default_mode".to_string(),
                params: Some(serde_json::to_value(SettingsSetDefaultModeParams { mode }).unwrap()),
            })
            .await;

        match decode_response::<SettingsSetDefaultModeResult>(res) {
            Ok(result) => {
                self.policy_mode = result.default_mode;
                self.timeline.push_system(format!(
                    "default mode set to {}.",
                    policy_mode_label(result.default_mode)
                ));
                self.push_event(format!(
                    "default mode selected: {}",
                    policy_mode_label(result.default_mode)
                ));
                self.show_provider_popup = false;
            }
            Err(err) => {
                self.record_error(format!("failed to set default mode: {err}"));
                self.show_provider_popup = false;
            }
        }
    }

    async fn set_file_backed_dynamic_context(&mut self, enabled: bool) {
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!(
                    "settings/set_file_backed_dynamic_context"
                )),
                method: "settings/set_file_backed_dynamic_context".to_string(),
                params: Some(
                    serde_json::to_value(SettingsSetFileBackedDynamicContextParams { enabled })
                        .unwrap(),
                ),
            })
            .await;

        match decode_response::<SettingsSetFileBackedDynamicContextResult>(res) {
            Ok(result) => {
                self.file_backed_dynamic_context = result.enabled;
                let state = if result.enabled {
                    "enabled"
                } else {
                    "disabled"
                };
                self.timeline
                    .push_system(format!("file-backed dynamic context {state}."));
                self.push_event(format!("file-backed dynamic context {state}"));
                self.show_provider_popup = false;
            }
            Err(err) => {
                self.record_error(format!("failed to set file-backed dynamic context: {err}"));
                self.show_provider_popup = false;
            }
        }
    }

    fn close_or_back_provider_popup(&mut self) {
        if !self.provider_menu_filter.is_empty() {
            self.provider_menu_filter.clear();
            self.clamp_provider_menu_selection();
            self.preview_highlighted_theme();
            return;
        }
        if self.provider_popup_screen == ProviderPopupScreen::Reasoning {
            self.open_models_submenu();
            return;
        }
        if self.provider_popup_screen == ProviderPopupScreen::ApiKey {
            self.pending_api_key_provider = None;
            self.open_providers_submenu();
            return;
        }
        if self.provider_popup_screen == ProviderPopupScreen::Settings {
            self.provider_popup_screen = ProviderPopupScreen::Main;
            self.provider_menu_items = main_provider_menu_items(&self.provider_choices);
            self.provider_state.select(Some(0));
            return;
        }
        if self.provider_popup_screen == ProviderPopupScreen::Runners {
            self.provider_popup_screen = ProviderPopupScreen::Main;
            self.provider_menu_items = main_provider_menu_items(&self.provider_choices);
            self.provider_state.select(Some(0));
            return;
        }
        if self.provider_popup_screen == ProviderPopupScreen::Spinner {
            self.provider_popup_screen = ProviderPopupScreen::Main;
            self.provider_menu_items = main_provider_menu_items(&self.provider_choices);
            self.provider_state.select(Some(0));
            return;
        }
        if self.provider_popup_screen == ProviderPopupScreen::WebSearch {
            self.provider_popup_screen = ProviderPopupScreen::Main;
            self.provider_menu_items = main_provider_menu_items(&self.provider_choices);
            self.provider_state.select(Some(0));
            return;
        }
        if self.provider_popup_screen == ProviderPopupScreen::VoiceModels {
            self.open_settings_submenu();
            self.provider_state.select(
                self.provider_menu_items
                    .iter()
                    .position(|item| matches!(item, ProviderMenuItem::VoiceModelSettings))
                    .or(Some(0)),
            );
            return;
        }
        if self.provider_popup_screen == ProviderPopupScreen::Shell {
            self.open_settings_submenu();
            return;
        }
        if self.provider_popup_screen == ProviderPopupScreen::Resume {
            self.provider_popup_screen = ProviderPopupScreen::Main;
            self.provider_menu_items = main_provider_menu_items(&self.provider_choices);
            self.provider_state.select(Some(0));
            return;
        }
        if self.provider_popup_screen == ProviderPopupScreen::Themes {
            // Leaving the themes screen without committing — revert any live
            // preview before returning to the main menu.
            self.cancel_theme_preview();
            self.provider_popup_screen = ProviderPopupScreen::Main;
            self.provider_menu_items = main_provider_menu_items(&self.provider_choices);
            self.provider_state.select(Some(0));
            return;
        }
        if self.provider_popup_screen == ProviderPopupScreen::Marketplaces {
            self.provider_popup_screen = ProviderPopupScreen::Main;
            self.provider_menu_items = main_provider_menu_items(&self.provider_choices);
            self.provider_state.select(Some(0));
            return;
        }
        if self.provider_popup_screen != ProviderPopupScreen::Main {
            self.provider_popup_screen = ProviderPopupScreen::Main;
            self.provider_menu_items = main_provider_menu_items(&self.provider_choices);
            self.provider_state.select(Some(0));
        } else {
            self.show_provider_popup = false;
        }
    }

    fn select_previous_provider_menu_item(&mut self) {
        self.select_provider_menu_item_delta(-1);
    }

    fn select_next_provider_menu_item(&mut self) {
        self.select_provider_menu_item_delta(1);
    }

    fn select_provider_menu_item_delta(&mut self, delta: isize) {
        let visible_items = self.filtered_provider_menu_items();
        if visible_items.is_empty() {
            self.provider_state.select(None);
            return;
        }
        let Some(mut i) = self.provider_state.selected() else {
            self.provider_state
                .select(first_selectable_provider_menu_index(&visible_items));
            self.preview_highlighted_theme();
            return;
        };
        i = i.min(visible_items.len() - 1);
        for _ in 0..visible_items.len() {
            i = if delta < 0 {
                if i == 0 {
                    visible_items.len() - 1
                } else {
                    i - 1
                }
            } else if i + 1 >= visible_items.len() {
                0
            } else {
                i + 1
            };
            if visible_items[i].is_selectable() {
                self.provider_state.select(Some(i));
                self.preview_highlighted_theme();
                return;
            }
        }
        self.provider_state.select(None);
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
            ProviderMenuItem::Providers => {
                self.open_providers_submenu();
            }
            ProviderMenuItem::Settings => {
                self.open_settings_submenu();
            }
            ProviderMenuItem::RoadmapMode => {
                self.show_provider_popup = false;
                self.enter_roadmap_mode(None);
            }
            ProviderMenuItem::RunnerSettings => {
                self.open_runners_submenu().await;
            }
            ProviderMenuItem::SpinnerSettings => {
                self.open_spinner_submenu();
            }
            ProviderMenuItem::WebSearchSettings => {
                self.open_web_search_submenu();
            }
            ProviderMenuItem::VoiceModelSettings => {
                self.open_voice_models_submenu().await;
            }
            ProviderMenuItem::ShellSettings(_) => {
                self.open_shell_submenu();
            }
            ProviderMenuItem::ThemesSettings => {
                self.open_themes_submenu();
            }
            ProviderMenuItem::MarketplacesSettings => {
                self.open_marketplaces_submenu();
            }
            ProviderMenuItem::PluginBrowser => {
                self.open_plugin_browser().await;
            }
            ProviderMenuItem::ResumeThreads => {
                self.open_resume_submenu().await;
            }
            ProviderMenuItem::Theme(id) => {
                self.select_theme(id);
            }
            ProviderMenuItem::MarketplaceInstallDefault { selection, .. } => {
                self.show_provider_popup = false;
                self.composer = composer_textarea(self.theme);
                self.composer
                    .insert_str(format!("/marketplace install-default {selection}"));
            }
            ProviderMenuItem::MarketplaceDefault { id, .. } => {
                self.show_provider_popup = false;
                self.composer = composer_textarea(self.theme);
                self.composer
                    .insert_str(format!("/marketplace refresh {id}"));
            }
            ProviderMenuItem::Spinner(spinner) => {
                self.select_working_spinner(spinner);
            }
            ProviderMenuItem::WebSearchMode(mode) => {
                self.set_web_search_mode(mode).await;
            }
            ProviderMenuItem::VoiceModel(choice) => {
                self.show_provider_popup = false;
                self.set_voice_model(choice.provider_id, choice.model_id);
            }
            ProviderMenuItem::ShellChoice(shell) => {
                self.set_command_shell(shell).await;
            }
            ProviderMenuItem::SearchIndexToggle(enabled) => {
                self.set_search_index_enabled(!enabled).await;
            }
            ProviderMenuItem::MessageFoldingToggle(enabled) => {
                self.set_timeline_message_folding(!enabled);
            }
            ProviderMenuItem::FileBackedDynamicContextToggle(enabled) => {
                self.set_file_backed_dynamic_context(!enabled).await;
            }
            ProviderMenuItem::DefaultMode(mode) => {
                self.set_default_mode(mode).await;
            }
            ProviderMenuItem::Provider(provider) => {
                self.select_provider(provider).await;
            }
            ProviderMenuItem::Model(option) => {
                self.select_provider_model(option).await;
            }
            ProviderMenuItem::Section(_) => {}
            ProviderMenuItem::Reasoning(option) => {
                self.select_provider_model_params(ProviderSelectParams {
                    provider: option.provider_id,
                    model: Some(option.model_id),
                    reasoning: Some(option.effort),
                    thread_id: Some(self.focused_thread_id().to_string()),
                })
                .await;
            }
            ProviderMenuItem::Runner {
                destination_id,
                provider_id,
                ..
            } => {
                self.show_provider_popup = false;
                self.select_runner(destination_id, provider_id).await;
            }
            ProviderMenuItem::Thread(thread) => {
                self.show_provider_popup = false;
                self.load_thread(thread.id).await;
            }
            ProviderMenuItem::Back => {
                self.close_or_back_provider_popup();
            }
        }
    }

    fn open_models_submenu(&mut self) {
        self.provider_popup_screen = ProviderPopupScreen::Models;
        self.provider_menu_filter.clear();
        self.provider_menu_items = models_menu_items(&self.model_options, &self.provider_choices);
        let selected = self
            .provider_menu_items
            .iter()
            .position(|item| {
                matches!(
                    item,
                    ProviderMenuItem::Model(option)
                        if option.provider_id == self.provider && option.model_id == self.model
                )
            })
            .or_else(|| first_selectable_provider_menu_index(&self.provider_menu_items));
        self.provider_state.select(selected);
    }

    fn open_providers_submenu(&mut self) {
        self.provider_popup_screen = ProviderPopupScreen::Providers;
        self.provider_menu_filter.clear();
        self.provider_menu_items = providers_menu_items(&self.provider_choices);
        let selected = self
            .provider_choices
            .iter()
            .position(|provider| provider.provider_id == self.provider)
            .unwrap_or(0);
        if self.provider_menu_items.is_empty() {
            self.provider_state.select(None);
        } else {
            self.provider_state.select(Some(selected));
        }
    }

    fn open_settings_submenu(&mut self) {
        self.provider_popup_screen = ProviderPopupScreen::Settings;
        self.provider_menu_filter.clear();
        self.provider_menu_items = settings_menu_items(
            self.timeline_settings,
            self.search_index_enabled,
            &self.command_shell,
            self.file_backed_dynamic_context,
        );
        let selected = self
            .provider_menu_items
            .iter()
            .position(|item| matches!(item, ProviderMenuItem::DefaultMode(mode) if *mode == self.policy_mode))
            .unwrap_or(0);
        self.provider_state.select(Some(selected));
    }

    async fn open_runners_submenu(&mut self) {
        self.provider_popup_screen = ProviderPopupScreen::Runners;
        self.provider_menu_filter.clear();
        match self.runners_list().await {
            Ok(runners) => {
                self.provider_menu_items = runner_menu_items(&runners);
                self.provider_state.select(Some(0));
            }
            Err(err) => {
                self.provider_menu_items = vec![ProviderMenuItem::Back];
                self.provider_state.select(Some(0));
                self.record_error(format!("runners/list failed: {err}"));
            }
        }
        self.show_provider_popup = true;
    }

    fn open_spinner_submenu(&mut self) {
        self.provider_popup_screen = ProviderPopupScreen::Spinner;
        self.provider_menu_filter.clear();
        self.provider_menu_items = WorkingSpinner::all()
            .iter()
            .copied()
            .map(ProviderMenuItem::Spinner)
            .chain(std::iter::once(ProviderMenuItem::Back))
            .collect();
        let selected = WorkingSpinner::all()
            .iter()
            .position(|spinner| *spinner == self.working_spinner)
            .unwrap_or(0);
        self.provider_state.select(Some(selected));
    }

    fn open_web_search_submenu(&mut self) {
        self.provider_popup_screen = ProviderPopupScreen::WebSearch;
        self.provider_menu_filter.clear();
        self.provider_menu_items = [
            HostedWebSearchMode::Cached,
            HostedWebSearchMode::Live,
            HostedWebSearchMode::Disabled,
        ]
        .into_iter()
        .map(ProviderMenuItem::WebSearchMode)
        .chain(std::iter::once(ProviderMenuItem::Back))
        .collect();
        let selected = self
            .provider_menu_items
            .iter()
            .position(|item| matches!(item, ProviderMenuItem::WebSearchMode(mode) if *mode == self.web_search_mode))
            .unwrap_or(0);
        self.provider_state.select(Some(selected));
    }

    async fn open_voice_models_submenu(&mut self) {
        self.provider_popup_screen = ProviderPopupScreen::VoiceModels;
        self.provider_menu_filter.clear();
        match self.speech_providers_list().await {
            Ok(providers) => {
                self.provider_menu_items = voice_model_menu_items(&providers);
                let selected = self
                    .provider_menu_items
                    .iter()
                    .position(|item| {
                        matches!(
                            item,
                            ProviderMenuItem::VoiceModel(choice)
                                if self.voice.provider() == Some(choice.provider_id.as_str())
                                    && self.voice.model() == Some(choice.model_id.as_str())
                        )
                    })
                    .or_else(|| first_selectable_provider_menu_index(&self.provider_menu_items));
                self.provider_state.select(selected);
            }
            Err(err) => {
                self.provider_menu_items = vec![ProviderMenuItem::Back];
                self.provider_state.select(Some(0));
                self.record_error(format!("speech/providers/list failed: {err}"));
            }
        }
        self.show_provider_popup = true;
    }

    fn open_shell_submenu(&mut self) {
        self.provider_popup_screen = ProviderPopupScreen::Shell;
        self.provider_menu_filter.clear();
        self.provider_menu_items = self
            .command_shell_options
            .iter()
            .cloned()
            .map(ProviderMenuItem::ShellChoice)
            .chain(std::iter::once(ProviderMenuItem::Back))
            .collect();
        let selected = self
            .provider_menu_items
            .iter()
            .position(|item| matches!(item, ProviderMenuItem::ShellChoice(shell) if shell == &self.command_shell))
            .unwrap_or(0);
        self.provider_state.select(Some(selected));
    }

    async fn open_resume_submenu(&mut self) {
        self.provider_popup_screen = ProviderPopupScreen::Resume;
        self.provider_menu_filter.clear();
        match thread_resume::threads_list(&self.client).await {
            Ok(threads) => {
                self.provider_menu_items = threads
                    .into_iter()
                    .map(Box::new)
                    .map(ProviderMenuItem::Thread)
                    .chain(std::iter::once(ProviderMenuItem::Back))
                    .collect();
                let selected = self
                    .provider_menu_items
                    .iter()
                    .position(|item| {
                        matches!(item, ProviderMenuItem::Thread(thread) if thread.id == self.thread_id)
                    })
                    .unwrap_or(0);
                self.provider_state.select(Some(selected));
            }
            Err(err) => {
                self.provider_menu_items = vec![ProviderMenuItem::Back];
                self.provider_state.select(Some(0));
                self.record_error(format!("thread/list failed: {err}"));
            }
        }
        self.show_provider_popup = true;
    }

    fn open_themes_submenu(&mut self) {
        // Snapshot the current theme so we can revert on Esc / Back. Only
        // snapshot when arriving from a non-Themes screen — re-entering
        // shouldn't clobber the original baseline.
        if self.provider_popup_screen != ProviderPopupScreen::Themes {
            self.theme_preview_baseline = Some((self.theme, self.active_theme_id.clone()));
        }
        self.provider_popup_screen = ProviderPopupScreen::Themes;
        self.provider_menu_filter.clear();
        let directories = crate::theme::discovery::default_directories();
        let entries = crate::theme::discover_themes(&directories);
        self.provider_menu_items = entries
            .iter()
            .map(|e| ProviderMenuItem::Theme(e.id.clone()))
            .chain(std::iter::once(ProviderMenuItem::Back))
            .collect();
        let selected = self
            .active_theme_id
            .as_deref()
            .and_then(|active| entries.iter().position(|e| e.id == active))
            .unwrap_or(0);
        if self.provider_menu_items.is_empty() {
            self.provider_state.select(None);
        } else {
            self.provider_state.select(Some(selected));
        }
        // Apply the initial highlight immediately so the user lands on a
        // surface that matches what their Enter would commit.
        self.preview_highlighted_theme();
    }

    fn open_marketplaces_submenu(&mut self) {
        self.provider_popup_screen = ProviderPopupScreen::Marketplaces;
        self.provider_menu_filter.clear();
        self.provider_menu_items = vec![
            ProviderMenuItem::PluginBrowser,
            ProviderMenuItem::MarketplaceInstallDefault {
                selection: "all",
                label: "Install all default marketplaces",
            },
            ProviderMenuItem::MarketplaceInstallDefault {
                selection: "anthropic",
                label: "Install Claude default marketplace",
            },
            ProviderMenuItem::MarketplaceInstallDefault {
                selection: "cursor",
                label: "Install Cursor default marketplace",
            },
            ProviderMenuItem::MarketplaceInstallDefault {
                selection: "codex",
                label: "Install Codex default marketplace",
            },
            ProviderMenuItem::Section("Default marketplace metadata".to_string()),
            ProviderMenuItem::MarketplaceDefault {
                id: "claude-plugins-official",
                kind: "Claude",
                label: "Anthropic Claude Plugins Official",
            },
            ProviderMenuItem::MarketplaceDefault {
                id: "cursor-plugins",
                kind: "Cursor",
                label: "Cursor Marketplace",
            },
            ProviderMenuItem::MarketplaceDefault {
                id: "codex-plugins",
                kind: "Codex",
                label: "Codex Plugins",
            },
            ProviderMenuItem::Back,
        ];
        self.provider_state
            .select(first_selectable_provider_menu_index(
                &self.provider_menu_items,
            ));
    }

    /// Apply the theme highlighted in the Themes submenu without persisting.
    /// Called after every navigation so the running TUI shows what the user
    /// is about to choose. No-op outside the Themes screen.
    fn preview_highlighted_theme(&mut self) {
        if self.provider_popup_screen != ProviderPopupScreen::Themes {
            return;
        }
        let Some(idx) = self.provider_state.selected() else {
            return;
        };
        let Some(item) = self.filtered_provider_menu_items().get(idx).cloned() else {
            return;
        };
        let ProviderMenuItem::Theme(id) = item else {
            // Hovering over the Back row — restore the baseline so the user
            // sees what they'd revert to.
            self.revert_theme_preview_in_place();
            return;
        };
        let directories = crate::theme::discovery::default_directories();
        if let Some(overrides) = crate::theme::load_theme_by_id(&directories, &id) {
            self.theme = Theme::for_terminal().with_overrides(&overrides);
            self.active_theme_id = Some(id);
        }
    }

    /// Restore the snapshot taken when the Themes submenu opened, but leave
    /// the snapshot in place — used while still navigating (e.g. hovering the
    /// Back row). For the final teardown use [`Self::cancel_theme_preview`].
    fn revert_theme_preview_in_place(&mut self) {
        if let Some((theme, id)) = &self.theme_preview_baseline {
            self.theme = *theme;
            self.active_theme_id = id.clone();
        }
    }

    /// Cancel an in-progress theme preview: restore the baseline and clear it.
    /// Safe to call when no preview is active.
    fn cancel_theme_preview(&mut self) {
        if let Some((theme, id)) = self.theme_preview_baseline.take() {
            self.theme = theme;
            self.active_theme_id = id;
        }
    }

    fn select_theme(&mut self, id: String) {
        let directories = crate::theme::discovery::default_directories();
        let Some(state_path) = crate::theme::state::state_file_path() else {
            self.record_error("could not resolve ~/.roder/state.toml".to_string());
            self.theme_preview_baseline = None;
            self.show_provider_popup = false;
            return;
        };
        match crate::theme::apply_theme(&directories, &state_path, &id) {
            Ok(overrides) => {
                self.theme = Theme::for_terminal().with_overrides(&overrides);
                self.active_theme_id = Some(id.clone());
                self.timeline.push_system(format!("theme set to {id}."));
                self.push_event(format!("theme applied: {id}"));
                // Commit: drop the baseline so any subsequent Esc out of a
                // different screen doesn't snap back to the previous theme.
                self.theme_preview_baseline = None;
            }
            Err(err) => {
                self.record_error(format!("failed to apply theme {id}: {err}"));
                // Failed to commit — revert to the baseline so the user
                // doesn't get stranded on a half-applied preview.
                self.cancel_theme_preview();
            }
        }
        self.show_provider_popup = false;
    }

    fn select_working_spinner(&mut self, spinner: WorkingSpinner) {
        self.working_spinner = spinner;
        match save_tui_spinner(spinner.id()) {
            Ok(()) => {
                self.push_event(format!("working spinner saved: {}", spinner.id()));
                self.timeline
                    .push_system(format!("working spinner set to {}.", spinner.label()));
            }
            Err(err) => {
                self.record_error(format!("failed to save working spinner: {err}"));
            }
        }
        self.show_provider_popup = false;
    }

    fn set_timeline_message_folding(&mut self, enabled: bool) {
        self.timeline_settings.message_folding = enabled;
        self.timeline.set_settings(self.timeline_settings);
        for timeline in self.team_timelines.values_mut() {
            timeline.set_settings(self.timeline_settings);
        }
        self.provider_menu_items = settings_menu_items(
            self.timeline_settings,
            self.search_index_enabled,
            &self.command_shell,
            self.file_backed_dynamic_context,
        );
        if let Some(selected) = self.provider_state.selected() {
            self.provider_state.select(Some(
                selected.min(self.provider_menu_items.len().saturating_sub(1)),
            ));
        }
        match save_tui_message_folding(enabled) {
            Ok(()) => {
                self.push_event(format!("message folding saved: {enabled}"));
                self.timeline.push_system(format!(
                    "long message folding {}.",
                    if enabled { "enabled" } else { "disabled" }
                ));
            }
            Err(err) => {
                self.record_error(format!(
                    "failed to save long message folding setting: {err}"
                ));
            }
        }
    }

    fn filtered_provider_menu_items(&self) -> Vec<ProviderMenuItem> {
        filter_provider_menu_items(&self.provider_menu_items, &self.provider_menu_filter)
    }

    fn clamp_provider_menu_selection(&mut self) {
        let visible_items = self.filtered_provider_menu_items();
        if visible_items.is_empty() {
            self.provider_state.select(None);
            return;
        }
        let selected = self
            .provider_state
            .selected()
            .unwrap_or(0)
            .min(visible_items.len() - 1);
        if visible_items[selected].is_selectable() {
            self.provider_state.select(Some(selected));
        } else {
            self.provider_state
                .select(first_selectable_provider_menu_index(&visible_items));
        }
    }

    async fn select_provider_model(&mut self, option: ProviderOption) {
        if !option.reasoning_options.is_empty() {
            self.open_reasoning_submenu(option);
            return;
        }
        let params = ProviderSelectParams {
            provider: option.provider_id,
            model: Some(option.model_id),
            reasoning: option.default_reasoning,
            thread_id: Some(self.focused_thread_id().to_string()),
        };
        self.select_provider_model_params(params).await;
    }

    fn open_reasoning_submenu(&mut self, option: ProviderOption) {
        self.provider_popup_screen = ProviderPopupScreen::Reasoning;
        self.provider_menu_filter.clear();
        self.pending_reasoning_model = Some(option.clone());
        self.provider_menu_items = option
            .reasoning_options
            .iter()
            .map(|reasoning| {
                ProviderMenuItem::Reasoning(ReasoningOptionChoice {
                    provider_id: option.provider_id.clone(),
                    model_id: option.model_id.clone(),
                    effort: reasoning.effort.clone(),
                    description: reasoning.description.clone(),
                })
            })
            .chain(std::iter::once(ProviderMenuItem::Back))
            .collect();
        let selected = option
            .reasoning_options
            .iter()
            .position(|reasoning| reasoning.effort == self.reasoning_effort)
            .or_else(|| {
                option.default_reasoning.as_ref().and_then(|default| {
                    option
                        .reasoning_options
                        .iter()
                        .position(|reasoning| &reasoning.effort == default)
                })
            })
            .unwrap_or(0);
        self.provider_state.select(Some(selected));
    }

    async fn select_provider(&mut self, provider: ProviderChoice) {
        if provider.auth_type == ProviderAuthType::ApiKey && !provider.authenticated {
            self.open_provider_api_key_prompt(provider);
            return;
        }
        if provider.auth_type == ProviderAuthType::OAuth && !provider.authenticated {
            let Some(auth_flow) = ProviderAuthFlow::for_provider(&provider.provider_id) else {
                self.record_error(format!(
                    "provider {} requires OAuth; no login flow is available",
                    provider.provider_id
                ));
                self.show_provider_popup = false;
                return;
            };
            if !self.run_provider_auth(auth_flow).await {
                return;
            }
        }
        let params = ProviderSelectParams {
            provider: provider.provider_id,
            model: provider.default_model,
            reasoning: None,
            thread_id: Some(self.focused_thread_id().to_string()),
        };
        self.select_provider_model_params(params).await;
    }

    fn open_provider_api_key_prompt(&mut self, provider: ProviderChoice) {
        self.provider_popup_screen = ProviderPopupScreen::ApiKey;
        self.provider_menu_filter.clear();
        self.provider_menu_items = Vec::new();
        let api_key_url = provider_api_key_url(&provider.provider_id);
        self.timeline.push_system(format!(
            "open {api_key_url} and copy an API key for {}.",
            provider.name,
        ));
        self.pending_api_key_provider = Some(provider);
        self.provider_state.select(None);
    }

    async fn open_provider_api_key_url(&mut self) {
        let Some(provider) = self.pending_api_key_provider.as_ref() else {
            return;
        };
        let url = provider_api_key_url(&provider.provider_id);
        match open_url(url).await {
            Ok(()) => self.push_event(format!("opened {} API keys", provider.name)),
            Err(err) => self.record_error(format!("failed to open {url}: {err}")),
        }
    }

    async fn submit_provider_api_key(&mut self) {
        let api_key = self.provider_menu_filter.trim().to_string();
        if api_key.is_empty() {
            self.record_error("API key is required.".to_string());
            return;
        }
        let Some(provider) = self.pending_api_key_provider.clone() else {
            self.close_or_back_provider_popup();
            return;
        };
        let provider_id = provider.provider_id.clone();
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("providers/configure")),
                method: "providers/configure".to_string(),
                params: Some(
                    serde_json::to_value(ProviderConfigureParams {
                        provider: provider_id.clone(),
                        api_key,
                    })
                    .unwrap(),
                ),
            })
            .await;
        match decode_response::<ProviderConfigureResult>(res) {
            Ok(_) => {
                self.timeline
                    .push_system(format!("configured API key for {}.", provider.name));
                self.push_event(format!("provider configured: {provider_id}"));
                self.provider_menu_filter.clear();
                self.pending_api_key_provider = None;
                self.open_provider_popup().await;
            }
            Err(err) => {
                self.record_error(format!("providers/configure failed: {err}"));
                self.show_provider_popup = false;
            }
        }
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
                self.reasoning_effort = selected.reasoning;
                self.model_context_window =
                    context_window_from_options(&self.model_options, &self.provider, &self.model)
                        .or_else(|| context_window_for_model(&self.model));
                self.timeline.push_system(format!(
                    "switched provider/model to {}/{} with reasoning {}.",
                    self.provider, self.model, self.reasoning_effort
                ));
                self.push_event(format!(
                    "provider selected: {}/{} ({})",
                    self.provider, self.model, self.reasoning_effort
                ));
                self.show_provider_popup = false;
                self.pending_reasoning_model = None;
            }
            Err(err) => {
                self.record_error(format!("providers/select failed: {err}"));
                self.show_provider_popup = false;
            }
        }
    }

    async fn run_provider_auth(&mut self, flow: ProviderAuthFlow) -> bool {
        self.timeline.push_system(format!(
            "opening browser for {} sign-in.",
            flow.display_name
        ));
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!(flow.login_method)),
                method: flow.login_method.to_string(),
                params: None,
            })
            .await;
        match decode_response::<ProviderAuthResult>(res) {
            Ok(result) => {
                self.timeline.push_system(
                    provider_auth_message(
                        flow.display_name,
                        flow.logout_method,
                        flow.login_method,
                        &result,
                    )
                    .replace("system: ", ""),
                );
                self.push_event(format!(
                    "{} auth: {}",
                    flow.provider_id,
                    provider_auth_event(&result)
                ));
                true
            }
            Err(err) => {
                self.record_error(format!("{} auth failed: {err}", flow.display_name));
                self.show_provider_popup = false;
                false
            }
        }
    }

    fn record_error(&mut self, message: String) {
        self.timeline.push_error(message.clone());
        self.push_event(format!("error: {message}"));
    }

    fn record_tool_requested_with_id(&mut self, tool_id: String, entry: ToolTimelineEntry) {
        if is_raw_tool_name(&entry.name) {
            self.tool_names.insert(tool_id.clone(), entry.name.clone());
        }
        if is_stdin_tool(&entry.name) {
            if let Some(session_id) = session_id_from_tool_arguments(&entry.arguments) {
                self.stdin_tool_sessions.insert(tool_id.clone(), session_id);
            }
            self.hidden_stdin_tools.insert(tool_id.clone());
            self.timeline.remove_tool(&tool_id);
            return;
        }
        self.timeline.record_tool_requested(tool_id, entry);
    }

    fn record_tool_delta(&mut self, tool_id: &str, arguments_delta: &str) {
        if self.hidden_stdin_tools.contains(tool_id) {
            return;
        }
        self.timeline.record_tool_delta(tool_id, arguments_delta);
    }

    fn record_tool_completed(&mut self, tool_id: &str, failed: bool, output: Option<String>) {
        let tool_name = self.tool_names.remove(tool_id);
        let hidden_stdin = self.hidden_stdin_tools.remove(tool_id);
        let raw_session_id = output.as_deref().and_then(session_id_from_tool_result);
        let raw_still_running = output.as_deref().is_some_and(tool_result_is_running);
        if hidden_stdin {
            if let Some(session_id) = self.stdin_tool_sessions.remove(tool_id).or(raw_session_id)
                && let Some(exec_tool_id) = self.exec_session_tools.get(&session_id).cloned()
            {
                let output_delta = output
                    .as_deref()
                    .and_then(aggregated_output_from_tool_result);
                if !raw_still_running {
                    self.exec_session_tools.remove(&session_id);
                }
                self.timeline.record_tool_session_update(
                    &exec_tool_id,
                    failed,
                    output_delta,
                    raw_still_running,
                );
                return;
            }
            return;
        }
        if tool_name.as_deref().is_some_and(is_stdin_tool) {
            self.stdin_tool_sessions.remove(tool_id);
        }
        if !failed
            && tool_name.as_deref() == Some("update_plan")
            && let Some(text) = output.as_deref()
        {
            self.plan_panel.replace_from_update_plan_output(text);
        }
        if !failed
            && tool_name.as_deref() == Some("task_ledger.update")
            && let Some(text) = output.as_deref()
        {
            self.plan_panel.replace_from_task_ledger_output(text);
        }
        let timeline_output = if tool_name
            .as_deref()
            .is_some_and(tool_timeline::is_shell_like_tool)
        {
            output
                .as_deref()
                .and_then(aggregated_output_from_tool_result)
                .or(output)
        } else {
            output
        };
        let exec_session_is_running = tool_name.as_deref().is_some_and(is_exec_session_tool)
            && raw_session_id.is_some()
            && raw_still_running;
        if exec_session_is_running && let Some(session_id) = raw_session_id {
            self.exec_session_tools
                .insert(session_id, tool_id.to_string());
        }
        self.timeline
            .record_tool_completed(tool_id, failed, timeline_output);
    }

    fn toggle_plan_panel(&mut self) {
        if self.plan_panel.is_empty() {
            return;
        }
        self.plan_panel.toggle();
        self.push_event(if self.plan_panel.is_visible() {
            "todos shown".to_string()
        } else {
            "todos hidden".to_string()
        });
    }

    fn record_compaction_progress(&mut self, status: &str) {
        match status {
            "started" => {
                if !self.compaction_active {
                    self.timeline.push_system("Compacting context...");
                }
                self.compaction_active = true;
            }
            "completed" => {
                if self.compaction_active {
                    self.timeline.push_system("Context compacted.");
                }
                self.compaction_active = false;
            }
            other => {
                self.timeline
                    .push_system(format!("Context compaction {other}."));
            }
        }
    }

    fn record_provider_metadata(&mut self, metadata: &serde_json::Value) {
        if let Some(tokens) = reasoning_tokens_from_provider_metadata(metadata) {
            self.current_turn_reasoning_tokens = Some(
                self.current_turn_reasoning_tokens
                    .unwrap_or_default()
                    .max(tokens),
            );
        }
    }

    fn record_usage(&mut self, usage: TokenUsage) {
        record_usage_counters(
            &mut self.current_turn_input_tokens,
            &mut self.current_turn_output_tokens,
            &mut self.current_turn_total_tokens,
            &mut self.thread_tokens,
            &mut self.context_window_tokens,
            usage,
        );
    }

    fn push_event(&mut self, event: String) {
        self.events.push(event);
        if self.events.len() > 12 {
            self.events.remove(0);
        }
    }
}

fn spinner_frame(spinner: WorkingSpinner, frame: u64) -> &'static str {
    let frames = spinner.frames();
    frames[(frame as usize) % frames.len()]
}

fn padded_spinner_frame(spinner: WorkingSpinner, frame: u64) -> String {
    let frame = spinner_frame(spinner, frame);
    let width = spinner_frame_width(spinner);
    format!("{frame:<width$}")
}

fn spinner_frame_width(spinner: WorkingSpinner) -> usize {
    spinner
        .frames()
        .iter()
        .map(|frame| frame.chars().count())
        .max()
        .unwrap_or(0)
}

fn top_status_animation_interval() -> Duration {
    Duration::from_nanos(1_000_000_000 / TOP_STATUS_ANIMATION_FPS)
}

fn top_status_animation_poll_timeout(next_tick: Instant, now: Instant) -> Duration {
    next_tick.saturating_duration_since(now)
}

fn advance_top_status_animation(frame: &mut u64, next_tick: &mut Instant, now: Instant) {
    if now < *next_tick {
        return;
    }
    *frame = frame.wrapping_add(1);
    *next_tick = now + top_status_animation_interval();
}

#[derive(Debug, Default)]
struct TuiUserConfig {
    spinner: Option<String>,
    scroll_acceleration: Option<TuiScrollAccelerationConfig>,
    scroll_speed: Option<f32>,
    timeline: Option<TuiTimelineConfig>,
    voice: Option<VoiceConfig>,
}

#[derive(Debug, Clone, Copy, Default)]
struct TuiScrollAccelerationConfig {
    enabled: Option<bool>,
}

#[derive(Debug, Clone, Copy, Default)]
struct TuiTimelineConfig {
    message_folding: Option<bool>,
}

impl TuiUserConfig {
    fn scroll_settings(&self) -> ScrollSettings {
        ScrollSettings {
            acceleration_enabled: self
                .scroll_acceleration
                .and_then(|config| config.enabled)
                .unwrap_or(true),
            fixed_rows_per_tick: self
                .scroll_speed
                .filter(|speed| speed.is_finite() && *speed >= 0.001)
                .unwrap_or_else(|| ScrollSettings::default().fixed_rows_per_tick),
        }
    }

    fn timeline_settings(&self) -> TimelineSettings {
        TimelineSettings {
            message_folding: self
                .timeline
                .and_then(|config| config.message_folding)
                .unwrap_or(false),
        }
    }
}

fn load_tui_config() -> anyhow::Result<TuiUserConfig> {
    let path = tui_config_path();
    if !path.exists() {
        return Ok(TuiUserConfig::default());
    }
    let contents = std::fs::read_to_string(path)?;
    let value = match toml::from_str::<toml::Value>(&contents) {
        Ok(value) => value,
        Err(_) => {
            return Ok(TuiUserConfig {
                voice: fallback_voice_config_from_text(&contents),
                ..TuiUserConfig::default()
            });
        }
    };
    Ok(TuiUserConfig {
        spinner: value
            .get("tui")
            .and_then(|tui| tui.get("spinner"))
            .and_then(|spinner| spinner.as_str())
            .map(str::to_string),
        scroll_acceleration: value
            .get("tui")
            .and_then(|tui| tui.get("scroll_acceleration"))
            .and_then(|config| config.as_table())
            .map(|config| TuiScrollAccelerationConfig {
                enabled: config.get("enabled").and_then(|enabled| enabled.as_bool()),
            }),
        scroll_speed: value
            .get("tui")
            .and_then(|tui| tui.get("scroll_speed"))
            .and_then(|speed| match speed {
                toml::Value::Float(value) if value.is_finite() => Some(*value as f32),
                toml::Value::Integer(value) => Some(*value as f32),
                _ => None,
            }),
        timeline: value
            .get("tui")
            .and_then(|tui| tui.get("timeline"))
            .and_then(|config| config.as_table())
            .map(|config| TuiTimelineConfig {
                message_folding: config
                    .get("message_folding")
                    .and_then(|enabled| enabled.as_bool()),
            }),
        voice: value
            .get("tui")
            .and_then(|tui| tui.get("voice"))
            .and_then(|config| config.as_table())
            .map(|config| VoiceConfig {
                enabled: config.get("enabled").and_then(|enabled| enabled.as_bool()),
                mode: config
                    .get("mode")
                    .and_then(|mode| mode.as_str())
                    .and_then(VoiceMode::from_config_value),
                record_command: config
                    .get("record_command")
                    .and_then(|command| command.as_str())
                    .map(str::to_string),
                provider: config
                    .get("provider")
                    .and_then(|provider| provider.as_str())
                    .map(str::to_string),
                model: config
                    .get("model")
                    .and_then(|model| model.as_str())
                    .map(str::to_string),
                language: config
                    .get("language")
                    .and_then(|language| language.as_str())
                    .map(str::to_string),
                mime_type: config
                    .get("mime_type")
                    .and_then(|mime_type| mime_type.as_str())
                    .map(str::to_string),
                hold_idle_stop_millis: config
                    .get("hold_idle_stop_millis")
                    .and_then(toml::Value::as_integer)
                    .and_then(|value| u64::try_from(value).ok()),
            }),
    })
}

fn fallback_voice_config_from_text(contents: &str) -> Option<VoiceConfig> {
    let values = fallback_toml_table_assignments(contents, "tui.voice");
    if values.is_empty() {
        return None;
    }
    Some(VoiceConfig {
        enabled: values.get("enabled").and_then(|value| match value.trim() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        }),
        mode: values
            .get("mode")
            .and_then(|value| fallback_toml_string(value))
            .and_then(VoiceMode::from_config_value),
        record_command: values
            .get("record_command")
            .and_then(|value| fallback_toml_string(value).map(str::to_string)),
        provider: values
            .get("provider")
            .and_then(|value| fallback_toml_string(value).map(str::to_string)),
        model: values
            .get("model")
            .and_then(|value| fallback_toml_string(value).map(str::to_string)),
        language: values
            .get("language")
            .and_then(|value| fallback_toml_string(value).map(str::to_string)),
        mime_type: values
            .get("mime_type")
            .and_then(|value| fallback_toml_string(value).map(str::to_string)),
        hold_idle_stop_millis: values
            .get("hold_idle_stop_millis")
            .and_then(|value| value.trim().parse::<u64>().ok()),
    })
}

fn fallback_toml_table_assignments<'a>(
    contents: &'a str,
    table_name: &str,
) -> HashMap<&'a str, &'a str> {
    let header = format!("[{table_name}]");
    let mut in_table = false;
    let mut values = HashMap::new();
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_table = trimmed == header;
            continue;
        }
        if !in_table {
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if !key.is_empty() && !key.starts_with('#') {
            values.insert(key, value.trim());
        }
    }
    values
}

fn fallback_toml_string(value: &str) -> Option<&str> {
    let value = value.trim();
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
        })
}

fn save_tui_spinner(spinner: &str) -> anyhow::Result<()> {
    save_tui_value(&["spinner"], toml::Value::String(spinner.to_string()))
}

fn save_tui_message_folding(enabled: bool) -> anyhow::Result<()> {
    save_tui_value(
        &["timeline", "message_folding"],
        toml::Value::Boolean(enabled),
    )
}

fn save_tui_value(path_segments: &[&str], saved_value: toml::Value) -> anyhow::Result<()> {
    let path = tui_config_path();
    let contents = if path.exists() {
        Some(std::fs::read_to_string(&path)?)
    } else {
        None
    };
    let mut value = if let Some(contents) = contents.as_deref() {
        match toml::from_str::<toml::Value>(contents)
            .and_then(|value| ensure_tui_config_table_shape(value, path_segments))
        {
            Ok(value) => value,
            Err(_err) => {
                let updated = patch_tui_config_text(contents, path_segments, saved_value)?;
                write_tui_config_file(&path, updated)?;
                return Ok(());
            }
        }
    } else {
        toml::Value::Table(toml::map::Map::new())
    };

    let root = value
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("config root must be a TOML table"))?;
    let tui = root
        .entry("tui".to_string())
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    let tui = tui
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("[tui] config must be a TOML table"))?;
    insert_nested_toml_value(tui, path_segments, saved_value)?;

    write_tui_config_file(&path, toml::to_string_pretty(&value)?)?;
    Ok(())
}

fn ensure_tui_config_table_shape(
    mut value: toml::Value,
    path_segments: &[&str],
) -> Result<toml::Value, toml::de::Error> {
    let Some(root) = value.as_table_mut() else {
        return toml::from_str("=");
    };
    if let Some(tui) = root.get_mut("tui")
        && !tui.is_table()
    {
        return toml::from_str("=");
    }
    let Some(tui) = root.get_mut("tui").and_then(toml::Value::as_table_mut) else {
        return Ok(value);
    };
    if nested_tui_path_conflicts(tui, path_segments) {
        return toml::from_str("=");
    }
    Ok(value)
}

fn nested_tui_path_conflicts(
    table: &toml::map::Map<String, toml::Value>,
    path_segments: &[&str],
) -> bool {
    let Some((first, rest)) = path_segments.split_first() else {
        return true;
    };
    if rest.is_empty() {
        return false;
    }
    match table.get(*first) {
        Some(toml::Value::Table(child)) => nested_tui_path_conflicts(child, rest),
        Some(_) => true,
        None => false,
    }
}

fn write_tui_config_file(path: &Path, contents: String) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, contents)?;
    std::fs::rename(tmp, path)?;
    Ok(())
}

fn insert_nested_toml_value(
    table: &mut toml::map::Map<String, toml::Value>,
    path_segments: &[&str],
    value: toml::Value,
) -> anyhow::Result<()> {
    let Some((first, rest)) = path_segments.split_first() else {
        return Err(anyhow::anyhow!("config key path must not be empty"));
    };
    if rest.is_empty() {
        table.insert((*first).to_string(), value);
        return Ok(());
    }
    let child = table
        .entry((*first).to_string())
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    let child = child
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("[tui.{first}] config must be a TOML table"))?;
    insert_nested_toml_value(child, rest, value)
}

fn patch_tui_config_text(
    contents: &str,
    path_segments: &[&str],
    value: toml::Value,
) -> anyhow::Result<String> {
    let Some((key, table_segments)) = path_segments.split_last() else {
        anyhow::bail!("config key path must not be empty");
    };
    let table_name = std::iter::once("tui")
        .chain(table_segments.iter().copied())
        .collect::<Vec<_>>()
        .join(".");
    let literal = toml_value_literal(value)?;
    let assignment = format!("{key} = {literal}");

    let mut lines = contents.lines().map(str::to_string).collect::<Vec<_>>();
    let header = format!("[{table_name}]");
    let section_start = lines
        .iter()
        .position(|line| line.trim() == header)
        .map(|index| index + 1);

    if let Some(start) = section_start {
        let end = lines[start..]
            .iter()
            .position(|line| {
                let trimmed = line.trim();
                trimmed.starts_with('[') && trimmed.ends_with(']')
            })
            .map(|offset| start + offset)
            .unwrap_or(lines.len());
        if let Some(existing) = lines[start..end]
            .iter()
            .position(|line| toml_assignment_key(line).is_some_and(|existing| existing == *key))
            .map(|offset| start + offset)
        {
            lines[existing] = assignment;
        } else {
            lines.insert(end, assignment);
        }
    } else {
        if !lines.last().is_none_or(|line| line.trim().is_empty()) {
            lines.push(String::new());
        }
        lines.push(header);
        lines.push(assignment);
    }

    let mut updated = lines.join("\n");
    updated.push('\n');
    Ok(updated)
}

fn toml_assignment_key(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') || trimmed.starts_with('[') {
        return None;
    }
    trimmed
        .split_once('=')
        .map(|(key, _)| key.trim())
        .filter(|key| !key.is_empty())
}

fn toml_value_literal(value: toml::Value) -> anyhow::Result<String> {
    match value {
        toml::Value::Boolean(value) => Ok(value.to_string()),
        toml::Value::String(value) => Ok(format!("{value:?}")),
        toml::Value::Integer(value) => Ok(value.to_string()),
        toml::Value::Float(value) if value.is_finite() => Ok(value.to_string()),
        other => anyhow::bail!("unsupported fallback TOML value: {other:?}"),
    }
}

fn tui_config_path() -> PathBuf {
    std::env::var_os("RODER_CONFIG_DIR")
        .or_else(|| std::env::var_os("RODER_DATA_DIR"))
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".roder")))
        .unwrap_or_else(|| PathBuf::from(".roder"))
        .join("config.toml")
}

fn format_working_elapsed(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    let hours = total_seconds / 3_600;
    let minutes = (total_seconds % 3_600) / 60;
    let seconds = total_seconds % 60;

    if hours > 0 {
        format!("{hours}h {minutes}m {seconds}s")
    } else if minutes > 0 {
        format!("{minutes}m {seconds}s")
    } else {
        format!("{seconds}s")
    }
}

fn working_status_label(compaction_active: bool) -> &'static str {
    if compaction_active {
        "Compacting context"
    } else {
        "Working"
    }
}

fn reasoning_tokens_from_provider_metadata(metadata: &serde_json::Value) -> Option<u32> {
    metadata
        .get("usage")
        .and_then(|usage| {
            usage
                .get("output_tokens_details")
                .or_else(|| usage.get("completion_tokens_details"))
        })
        .and_then(|details| {
            details
                .get("reasoning_tokens")
                .or_else(|| details.get("thinking_tokens"))
        })
        .and_then(serde_json::Value::as_u64)
        .and_then(|tokens| u32::try_from(tokens).ok())
}

fn event_log_height(show_event_log: bool, event_count: usize) -> u16 {
    if show_event_log {
        (event_count as u16 + 2).clamp(3, 8)
    } else {
        0
    }
}

#[cfg(test)]
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

fn queued_prompt_height(count: usize) -> u16 {
    if count == 0 {
        0
    } else {
        count.min(3) as u16 + 1 + u16::from(count > 3)
    }
}

fn slash_command_suffix(args: &str) -> String {
    if args.trim().is_empty() {
        String::new()
    } else {
        format!(" {}", args.trim())
    }
}

fn copied_helper_widget(theme: Theme) -> Paragraph<'static> {
    Paragraph::new(Line::from(vec![
        Span::styled("✓ ", theme.accent()),
        Span::styled(COPIED_HELPER_LABEL, theme.muted()),
    ]))
}

fn copied_helper_area(composer_area: Rect) -> Option<Rect> {
    if composer_area.y == 0 || composer_area.width == 0 {
        return None;
    }

    let width = copied_helper_width().min(composer_area.width);
    Some(Rect::new(
        composer_area.x + composer_area.width.saturating_sub(width + 2),
        composer_area.y - 1,
        width,
        1,
    ))
}

fn copied_helper_width() -> u16 {
    (2 + COPIED_HELPER_LABEL.chars().count()) as u16
}

fn voice_transcribing_widget(
    theme: Theme,
    spinner: WorkingSpinner,
    frame: u64,
) -> Paragraph<'static> {
    Paragraph::new(Line::from(vec![
        Span::styled(
            format!(" {} ", padded_spinner_frame(spinner, frame)),
            theme.running(),
        ),
        Span::styled("transcribing...", theme.muted()),
    ]))
}

fn voice_helper_area(composer_area: Rect) -> Option<Rect> {
    if composer_area.width == 0 {
        return None;
    }
    let width = voice_helper_width().min(composer_area.width);
    Some(Rect::new(
        composer_area.x + composer_area.width.saturating_sub(width + 2),
        composer_area.y,
        width,
        1,
    ))
}

fn voice_helper_width() -> u16 {
    20
}

fn rect_contains(area: Rect, column: u16, row: u16) -> bool {
    row >= area.y
        && row < area.y.saturating_add(area.height)
        && column >= area.x
        && column < area.x.saturating_add(area.width)
}

fn selectable_lines_from_text(text: &Text<'_>, area: Rect, scroll: u16) -> Vec<SelectableLine> {
    rendered_selectable_rows(text, area, scroll)
        .into_iter()
        .map(|row| SelectableLine {
            row: row.row,
            text: row.text,
        })
        .collect()
}

fn highlight_selection(
    text: Text<'static>,
    area: Rect,
    scroll: u16,
    selection: &MouseSelection,
    theme: Theme,
) -> Text<'static> {
    let range = normalized_selection(selection);
    let render_height = scroll.saturating_add(area.height);
    let rows = rendered_text_rows(&text, area.width, render_height, 0);
    let lines = rows
        .into_iter()
        .enumerate()
        .map(|(index, row)| {
            let row_number = u16::try_from(index).unwrap_or(u16::MAX);
            let screen_row = area.y.saturating_add(row_number.saturating_sub(scroll));
            if row_number < scroll || row_number >= scroll.saturating_add(area.height) {
                return row.into_line(None, theme);
            }
            let selected = selection_columns_for_row(screen_row, &row.text(), range);
            row.into_line(selected, theme)
        })
        .collect::<Vec<_>>();
    Text::from(lines)
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct RenderedSelectableRow {
    row: u16,
    text: String,
    cells: Vec<RenderedCell>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct RenderedCell {
    symbol: String,
    style: Style,
}

impl RenderedSelectableRow {
    fn into_line(self, selected: Option<(usize, usize)>, theme: Theme) -> Line<'static> {
        let spans = self
            .cells
            .into_iter()
            .enumerate()
            .map(|(index, cell)| {
                let style = if selected.is_some_and(|(start, end)| (start..end).contains(&index)) {
                    theme.selected()
                } else {
                    cell.style
                };
                Span::styled(cell.symbol, style)
            })
            .collect::<Vec<_>>();
        Line::from(spans)
    }

    fn text(&self) -> String {
        self.cells.iter().map(|cell| cell.symbol.as_str()).collect()
    }
}

fn rendered_selectable_rows(
    text: &Text<'_>,
    area: Rect,
    scroll: u16,
) -> Vec<RenderedSelectableRow> {
    if area.is_empty() {
        return Vec::new();
    }

    rendered_text_rows(text, area.width, area.height, scroll)
        .into_iter()
        .enumerate()
        .filter_map(|(offset, mut row)| {
            (!row.text.trim().is_empty()).then(|| {
                row.row = area
                    .y
                    .saturating_add(u16::try_from(offset).unwrap_or(u16::MAX));
                row
            })
        })
        .collect()
}

fn rendered_text_rows(
    text: &Text<'_>,
    width: u16,
    height: u16,
    scroll: u16,
) -> Vec<RenderedSelectableRow> {
    if width == 0 || height == 0 {
        return Vec::new();
    }

    let render_area = Rect::new(0, 0, width, height);
    let mut buffer = Buffer::empty(render_area);
    Paragraph::new(text.clone())
        .scroll((scroll, 0))
        .wrap(Wrap { trim: false })
        .render(render_area, &mut buffer);

    (0..height)
        .map(|row| {
            let cells = buffer_row_cells(&buffer, row);
            let text = cells.iter().map(|cell| cell.symbol.as_str()).collect();
            RenderedSelectableRow { row, text, cells }
        })
        .collect()
}

fn buffer_row_cells(buffer: &Buffer, row: u16) -> Vec<RenderedCell> {
    let mut cells = (0..buffer.area.width)
        .map(|column| {
            let cell = &buffer[(column, row)];
            RenderedCell {
                symbol: cell.symbol().to_string(),
                style: cell.style(),
            }
        })
        .collect::<Vec<_>>();
    while cells.last().is_some_and(|cell| cell.symbol == " ") {
        cells.pop();
    }
    cells
}

fn selected_text(lines: &[SelectableLine], selection: &MouseSelection) -> Option<String> {
    let range = normalized_selection(selection);
    let selected = lines
        .iter()
        .filter_map(|line| {
            let (start, end) = selection_columns_for_row(line.row, &line.text, range)?;
            Some(slice_chars(&line.text, start, end).trim_end().to_string())
        })
        .collect::<Vec<_>>();
    let text = selected.join("\n");
    (!text.trim().is_empty()).then_some(text)
}

fn normalized_selection(selection: &MouseSelection) -> (SelectionPoint, SelectionPoint) {
    let a = selection.anchor;
    let b = selection.cursor;
    if (b.row, b.column) < (a.row, a.column) {
        (b, a)
    } else {
        (a, b)
    }
}

fn selection_columns_for_row(
    row: u16,
    text: &str,
    (start, end): (SelectionPoint, SelectionPoint),
) -> Option<(usize, usize)> {
    if row < start.row || row > end.row {
        return None;
    }
    let len = text.chars().count();
    let from = if row == start.row {
        usize::from(start.column).min(len)
    } else {
        0
    };
    let to = if row == end.row {
        usize::from(end.column).saturating_add(1).min(len)
    } else {
        len
    };
    (from < to).then_some((from, to))
}

fn slice_chars(text: &str, start: usize, end: usize) -> String {
    text.chars()
        .skip(start)
        .take(end.saturating_sub(start))
        .collect()
}

fn slash_command_menu_height<T>(matches: Option<&[T]>) -> u16 {
    let Some(matches) = matches else {
        return 0;
    };
    if matches.is_empty() {
        0
    } else {
        1 + matches.len().min(MAX_VISIBLE_SLASH_COMMANDS) as u16
    }
}

fn slash_command_preview_height<T>(matches: Option<&[T]>) -> u16 {
    matches
        .filter(|matches| !matches.is_empty())
        .map(|_| 1)
        .unwrap_or_default()
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
        path_from_file_uri(uri)?
    } else {
        expand_home_path(token)
    };
    is_image_path(&path).then_some(path)
}

fn path_from_file_uri(uri: &str) -> Option<PathBuf> {
    let decoded = percent_decode(uri)?;
    #[cfg(windows)]
    {
        let path = decoded.strip_prefix('/').filter(|path| {
            let bytes = path.as_bytes();
            bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
        });
        Some(PathBuf::from(path.unwrap_or(&decoded)))
    }
    #[cfg(not(windows))]
    {
        Some(PathBuf::from(decoded))
    }
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
                if chars.peek().is_some_and(|next| shell_escape_char(*next)) {
                    let next = chars.next().expect("peeked next shell escape");
                    current.push(next);
                } else {
                    current.push(ch);
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

fn shell_escape_char(ch: char) -> bool {
    ch.is_whitespace() || matches!(ch, '\\' | '\'' | '"')
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

fn image_inputs_from_attachments(
    attachments: &[ImageAttachment],
) -> anyhow::Result<Vec<InputImage>> {
    attachments
        .iter()
        .map(|attachment| {
            let bytes = std::fs::read(&attachment.path)
                .map_err(|err| anyhow::anyhow!("{}: {err}", attachment.path.display()))?;
            let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
            Ok(InputImage {
                image_url: format!(
                    "data:{};base64,{encoded}",
                    mime_type_for_image_path(&attachment.path)
                ),
            })
        })
        .collect()
}

fn mime_type_for_image_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("bmp") => "image/bmp",
        Some("tif" | "tiff") => "image/tiff",
        Some("heic") => "image/heic",
        Some("heif") => "image/heif",
        Some("avif") => "image/avif",
        Some("svg") => "image/svg+xml",
        _ => "application/octet-stream",
    }
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

fn aggregated_output_from_tool_result(output: &str) -> Option<String> {
    let marker = "Output:\n";
    let (_, tail) = output.split_once(marker)?;
    let text = tail.trim_end().to_string();
    (!text.is_empty()).then_some(text)
}

fn session_id_from_tool_result(output: &str) -> Option<u64> {
    output.lines().find_map(|line| {
        line.trim()
            .strip_prefix("Session id:")
            .and_then(|value| value.trim().parse::<u64>().ok())
    })
}

fn session_id_from_tool_arguments(arguments: &str) -> Option<u64> {
    serde_json::from_str::<serde_json::Value>(arguments)
        .ok()?
        .get("session_id")?
        .as_u64()
}

fn tool_result_is_running(output: &str) -> bool {
    output
        .lines()
        .any(|line| line.trim().eq_ignore_ascii_case("Status: running"))
}

fn is_stdin_tool(name: &str) -> bool {
    let normalized = name.to_ascii_lowercase();
    normalized == "write_stdin"
        || normalized.ends_with(".write_stdin")
        || normalized.ends_with("_write_stdin")
}

fn is_exec_session_tool(name: &str) -> bool {
    let normalized = name.to_ascii_lowercase();
    normalized == "exec_command"
        || normalized.ends_with(".exec_command")
        || normalized.ends_with("_exec_command")
}

async fn copy_selection_to_clipboards(text: String) -> anyhow::Result<()> {
    let mut errors = Vec::new();
    if let Err(err) = copy_to_system_clipboard(&text).await {
        errors.push(err.to_string());
    }
    if std::env::var_os("TMUX").is_some()
        && let Err(err) = copy_to_tmux_buffer(&text).await
    {
        errors.push(err.to_string());
    }
    if errors.is_empty() {
        Ok(())
    } else {
        anyhow::bail!(errors.join("; "))
    }
}

async fn copy_to_system_clipboard(text: &str) -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    let candidates: &[(&str, &[&str])] = &[("pbcopy", &[])];
    #[cfg(all(unix, not(target_os = "macos")))]
    let candidates: &[(&str, &[&str])] = &[
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard"]),
        ("xsel", &["--clipboard", "--input"]),
    ];
    #[cfg(windows)]
    let candidates: &[(&str, &[&str])] = &[("clip", &[])];

    for (program, args) in candidates {
        if pipe_to_command(program, args, text).await.is_ok() {
            return Ok(());
        }
    }
    anyhow::bail!("no clipboard command available")
}

async fn copy_to_tmux_buffer(text: &str) -> anyhow::Result<()> {
    pipe_to_command("tmux", &["load-buffer", "-"], text).await
}

async fn pipe_to_command(program: &str, args: &[&str], text: &str) -> anyhow::Result<()> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(text.as_bytes()).await?;
    }
    let status = child.wait().await?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("{program} exited with {status}")
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
                label: provider_model_label(&provider.id, &list.active_model),
                context_window: context_window_for_model(&list.active_model),
                default_reasoning: Some(list.active_reasoning.clone()),
                reasoning_options: Vec::new(),
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
                label: provider_model_label(&provider.id, &model_name),
                context_window: model
                    .context_window
                    .or_else(|| context_window_for_model(&model.id)),
                default_reasoning: model.default_reasoning.clone(),
                reasoning_options: model.supported_reasoning.clone(),
            });
        }
    }
    options
}

fn provider_model_label(provider_id: &str, model_name: &str) -> String {
    if model_name.starts_with(&format!("{provider_id}/")) {
        model_name.to_string()
    } else {
        format!("{provider_id}/{model_name}")
    }
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
    let _provider_count = providers.len();
    vec![
        ProviderMenuItem::Models,
        ProviderMenuItem::Providers,
        ProviderMenuItem::Settings,
        ProviderMenuItem::RoadmapMode,
        ProviderMenuItem::RunnerSettings,
        ProviderMenuItem::ResumeThreads,
        ProviderMenuItem::WebSearchSettings,
        ProviderMenuItem::SpinnerSettings,
        ProviderMenuItem::ThemesSettings,
        ProviderMenuItem::MarketplacesSettings,
    ]
}

fn runner_menu_items(runners: &RunnersListResult) -> Vec<ProviderMenuItem> {
    runners
        .providers
        .iter()
        .map(|provider| {
            let destination_id = provider.provider_id.clone();
            let provider_id = provider.provider_id.clone();
            ProviderMenuItem::Runner {
                destination_id,
                provider_id,
                label: runner_menu_label(provider, runners.active.as_ref()),
            }
        })
        .chain(std::iter::once(ProviderMenuItem::Back))
        .collect()
}

fn runner_menu_label(
    provider: &roder_protocol::RunnerProviderDescriptor,
    active: Option<&roder_protocol::RunnerStatus>,
) -> String {
    let active_suffix = if active.is_some_and(|status| {
        status.provider_id == provider.provider_id && status.destination_id == provider.provider_id
    }) {
        " (active)"
    } else {
        ""
    };
    format!(
        "{}{} - {}",
        provider.provider_id,
        active_suffix,
        runner_capabilities_label(&provider.capabilities)
    )
}

fn runner_capabilities_label(
    capabilities: &roder_api::remote_runner::RunnerCapabilities,
) -> String {
    let mut labels = Vec::new();
    if capabilities.command_exec {
        labels.push("commands");
    }
    if capabilities.file_read || capabilities.file_write {
        labels.push("files");
    }
    if capabilities.port_preview {
        labels.push("ports");
    }
    if capabilities.snapshots {
        labels.push("snapshots");
    }
    if capabilities.cancellation {
        labels.push("cancel");
    }
    if labels.is_empty() {
        "no capabilities".to_string()
    } else {
        labels.join(", ")
    }
}

fn providers_menu_items(providers: &[ProviderChoice]) -> Vec<ProviderMenuItem> {
    providers
        .iter()
        .cloned()
        .map(ProviderMenuItem::Provider)
        .chain(std::iter::once(ProviderMenuItem::Back))
        .collect()
}

fn settings_menu_items(
    timeline_settings: TimelineSettings,
    search_index_enabled: bool,
    command_shell: &str,
    file_backed_dynamic_context: bool,
) -> Vec<ProviderMenuItem> {
    [
        PolicyMode::Default,
        PolicyMode::AcceptAll,
        PolicyMode::Plan,
        PolicyMode::Bypass,
    ]
    .into_iter()
    .map(ProviderMenuItem::DefaultMode)
    .chain([
        ProviderMenuItem::SearchIndexToggle(search_index_enabled),
        ProviderMenuItem::ShellSettings(command_shell.to_string()),
        ProviderMenuItem::VoiceModelSettings,
        ProviderMenuItem::FileBackedDynamicContextToggle(file_backed_dynamic_context),
        ProviderMenuItem::MessageFoldingToggle(timeline_settings.message_folding),
        ProviderMenuItem::Back,
    ])
    .collect()
}

fn models_menu_items(
    models: &[ProviderOption],
    providers: &[ProviderChoice],
) -> Vec<ProviderMenuItem> {
    let mut items = Vec::new();
    let mut grouped_provider_ids = HashSet::new();

    for provider in providers {
        let provider_models = models
            .iter()
            .filter(|model| model.provider_id == provider.provider_id)
            .cloned()
            .collect::<Vec<_>>();
        if provider_models.is_empty() {
            continue;
        }

        grouped_provider_ids.insert(provider.provider_id.clone());
        items.push(ProviderMenuItem::Section(provider.name.clone()));
        items.extend(provider_models.into_iter().map(ProviderMenuItem::Model));
    }

    for model in models {
        if grouped_provider_ids.contains(&model.provider_id) {
            continue;
        }
        let provider_id = model.provider_id.clone();
        let provider_models = models
            .iter()
            .filter(|candidate| candidate.provider_id == provider_id)
            .cloned()
            .collect::<Vec<_>>();

        grouped_provider_ids.insert(provider_id.clone());
        items.push(ProviderMenuItem::Section(provider_id));
        items.extend(provider_models.into_iter().map(ProviderMenuItem::Model));
    }

    items.push(ProviderMenuItem::Back);
    items
}

fn voice_model_menu_items(providers: &SpeechProvidersListResult) -> Vec<ProviderMenuItem> {
    let mut items = Vec::new();
    for provider in &providers.providers {
        if provider.models.is_empty() {
            continue;
        }
        items.push(ProviderMenuItem::Section(provider.name.clone()));
        items.extend(provider.models.iter().map(|model| {
            ProviderMenuItem::VoiceModel(VoiceModelChoice {
                provider_id: provider.id.clone(),
                model_id: model.id.clone(),
                label: format!("{} / {}", provider.name, model.name),
            })
        }));
    }
    items.push(ProviderMenuItem::Back);
    items
}

fn filter_provider_menu_items(items: &[ProviderMenuItem], query: &str) -> Vec<ProviderMenuItem> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return items.to_vec();
    }
    items
        .iter()
        .filter(|item| item.is_selectable() && item.label().to_lowercase().contains(&query))
        .cloned()
        .collect()
}

fn first_selectable_provider_menu_index(items: &[ProviderMenuItem]) -> Option<usize> {
    items.iter().position(ProviderMenuItem::is_selectable)
}

fn provider_search_line(query: &str, theme: Theme) -> Line<'static> {
    let query = query.trim();
    let value = if query.is_empty() {
        Span::styled("type to filter", theme.muted())
    } else {
        Span::styled(query.to_string(), theme.text())
    };
    Line::from(vec![
        Span::styled(" / ", theme.accent()),
        value,
        Span::styled("  ", theme.muted()),
    ])
}

fn provider_api_key_input_line(query: &str, theme: Theme) -> Line<'static> {
    let value = if query.trim().is_empty() {
        Span::styled("paste API key", theme.muted())
    } else {
        Span::styled("[api key hidden]", theme.text())
    };
    Line::from(vec![
        Span::styled(" key ", theme.accent()),
        value,
        Span::styled("  enter save", theme.muted()),
    ])
}

fn provider_api_key_items(
    provider: Option<&ProviderChoice>,
    query: &str,
    theme: Theme,
) -> Vec<ListItem<'static>> {
    let provider_name = provider
        .map(|provider| provider.name.clone())
        .unwrap_or_else(|| "provider".to_string());
    let key_status = if query.trim().is_empty() {
        "waiting for key"
    } else {
        "key pasted"
    };
    vec![
        ListItem::new(Line::from(vec![
            Span::styled("Open ", theme.text()),
            Span::styled(
                provider
                    .map(|provider| provider_api_key_url(&provider.provider_id))
                    .unwrap_or("https://opencode.ai/auth"),
                theme.accent(),
            ),
        ])),
        ListItem::new(Line::from(Span::styled(
            format!("Create or copy a {provider_name} API key, then paste it here."),
            theme.text(),
        ))),
        ListItem::new(Line::from(vec![
            Span::styled("Ctrl+O", theme.accent()),
            Span::styled(" open page  ", theme.muted()),
            Span::styled("Enter", theme.accent()),
            Span::styled(" save  ", theme.muted()),
            Span::styled("Esc", theme.accent()),
            Span::styled(" back", theme.muted()),
        ])),
        ListItem::new(Line::from(Span::styled(key_status, theme.muted()))),
    ]
}

fn provider_api_key_url(provider_id: &str) -> &'static str {
    match provider_id {
        "cursor" => "https://cursor.com/dashboard/integrations",
        "poolside" => "https://platform.poolside.ai/api-keys",
        _ => "https://opencode.ai/auth",
    }
}

fn provider_auth_message(
    display_name: &str,
    logout_method: &str,
    method: &str,
    result: &ProviderAuthResult,
) -> String {
    match (method, result.signed_in, result.account_id.as_deref()) {
        (method, _, _) if method == logout_method => {
            format!("system: signed out of {display_name}.")
        }
        (_, true, Some(account_id)) => {
            format!("system: signed in with {display_name} account {account_id}.")
        }
        (_, true, None) => format!("system: signed in with {display_name}."),
        _ => format!("system: signed out of {display_name}."),
    }
}

fn provider_auth_event(result: &ProviderAuthResult) -> &'static str {
    if result.signed_in {
        "signed in"
    } else {
        "signed out"
    }
}

async fn open_url(url: &str) -> anyhow::Result<()> {
    let mut command = if cfg!(target_os = "macos") {
        let mut command = Command::new("open");
        command.arg(url);
        command
    } else if cfg!(target_os = "windows") {
        let mut command = Command::new("cmd");
        command.args(["/C", "start", "", url]);
        command
    } else {
        let mut command = Command::new("xdg-open");
        command.arg(url);
        command
    };
    command.spawn()?.wait().await?;
    Ok(())
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

async fn thread_state<C>(client: &C) -> anyhow::Result<ThreadStateResult>
where
    C: AppClient,
{
    let res = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("thread/state")),
            method: "thread/state".to_string(),
            params: None,
        })
        .await;
    decode_response(res)
}

async fn team_read<C>(client: &C, team_id: &str) -> anyhow::Result<TeamReadResult>
where
    C: AppClient,
{
    let res = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("team/read")),
            method: "team/read".to_string(),
            params: Some(
                serde_json::to_value(TeamReadParams {
                    team_id: team_id.to_string(),
                })
                .unwrap(),
            ),
        })
        .await;
    decode_response(res)
}

async fn settings_get<C>(client: &C) -> anyhow::Result<SettingsGetResult>
where
    C: AppClient,
{
    let res = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("settings/get")),
            method: "settings/get".to_string(),
            params: None,
        })
        .await;
    decode_response(res)
}

fn default_shell_settings() -> ShellSettings {
    let shell = roder_api::command_shell::default_command_shell();
    ShellSettings {
        options: roder_api::command_shell::command_shell_options(&shell),
        shell,
    }
}

fn next_policy_mode(mode: PolicyMode) -> PolicyMode {
    match mode {
        PolicyMode::Default => PolicyMode::AcceptAll,
        PolicyMode::AcceptAll => PolicyMode::Plan,
        PolicyMode::Plan | PolicyMode::Bypass => PolicyMode::Default,
    }
}

fn policy_mode_label(mode: PolicyMode) -> &'static str {
    match mode {
        PolicyMode::Default => "default",
        PolicyMode::AcceptAll => "accept_all",
        PolicyMode::Plan => "plan",
        PolicyMode::Bypass => "bypass",
    }
}

fn web_search_mode_label(mode: HostedWebSearchMode) -> &'static str {
    match mode {
        HostedWebSearchMode::Disabled => "Disabled",
        HostedWebSearchMode::Cached => "Cached hosted",
        HostedWebSearchMode::Live => "Live hosted",
    }
}

fn pretty_policy_mode_label(mode: PolicyMode) -> &'static str {
    match mode {
        PolicyMode::Default => "Default",
        PolicyMode::AcceptAll => "Accept All",
        PolicyMode::Plan => "Plan",
        PolicyMode::Bypass => "Bypass",
    }
}

fn settings_policy_mode_label(mode: PolicyMode) -> &'static str {
    match mode {
        PolicyMode::Default => "Default",
        PolicyMode::AcceptAll => "Accept edits",
        PolicyMode::Plan => "Plan",
        PolicyMode::Bypass => "Bypass",
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

#[cfg(test)]
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
    if let Some(body) = message.strip_prefix("tool_running: ") {
        return simple_message_lines("◆ ", body, theme.tool(), theme.muted());
    }
    if let Some(body) = message.strip_prefix("tool_failed: ") {
        return simple_message_lines("◆ ", body, theme.error(), theme.error());
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

#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
fn body_lines(body: &str) -> impl Iterator<Item = String> + '_ {
    body.split('\n').map(str::to_string)
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct ContextWindowCounter {
    used_tokens: u64,
    max_tokens: u64,
    hovered: bool,
}

impl ContextWindowCounter {
    fn label(self) -> String {
        if self.hovered {
            return self.expanded_label();
        }
        format!("│ {:.2}% │", self.percent())
    }

    fn spans(self, theme: Theme) -> Vec<Span<'static>> {
        if self.hovered {
            return vec![Span::styled(self.label(), theme.muted())];
        }
        let cells = 5usize;
        let filled = if self.used_tokens == 0 {
            0
        } else {
            (((self.percent() / 100.0) * cells as f64).ceil() as usize).clamp(1, cells)
        };
        vec![
            Span::styled("│ ", theme.muted()),
            Span::styled("▰".repeat(filled), theme.subtle()),
            Span::styled("▱".repeat(cells.saturating_sub(filled)), theme.muted()),
            Span::styled(format!(" {:.2}% │", self.percent()), theme.muted()),
        ]
    }

    fn percent(self) -> f64 {
        if self.max_tokens == 0 {
            return 0.0;
        }
        (self.used_tokens as f64 / self.max_tokens as f64) * 100.0
    }

    fn expanded_label(self) -> String {
        format!(
            "│ {} / {} │",
            compact_token_count(self.used_tokens),
            compact_token_count(self.max_tokens)
        )
    }

    fn hit_test(self, width: u16, row: u16, column: u16) -> bool {
        row == 0 && column >= width.saturating_sub(self.expanded_label().chars().count() as u16)
    }
}

fn compact_token_count(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        let floored_tenths = tokens / 100_000;
        return format!("{}.{}M", floored_tenths / 10, floored_tenths % 10);
    }
    if tokens >= 1_000 {
        return format!("{}K", tokens / 1_000);
    }
    tokens.to_string()
}

fn context_window_for_model(model: &str) -> Option<u32> {
    lookup_model(model).and_then(|entry| (entry.context_window > 0).then_some(entry.context_window))
}

fn context_window_from_options(
    options: &[ProviderOption],
    provider: &str,
    model: &str,
) -> Option<u32> {
    options
        .iter()
        .find(|option| option.provider_id == provider && option.model_id == model)
        .and_then(|option| option.context_window)
}

fn record_usage_counters(
    current_turn_input_tokens: &mut u32,
    current_turn_output_tokens: &mut u32,
    current_turn_total_tokens: &mut u32,
    thread_tokens: &mut u64,
    context_window_tokens: &mut u64,
    usage: TokenUsage,
) {
    let total_tokens = usage
        .total_tokens
        .max(usage.prompt_tokens.saturating_add(usage.completion_tokens));

    *current_turn_input_tokens = (*current_turn_input_tokens).max(usage.prompt_tokens);
    *current_turn_output_tokens = (*current_turn_output_tokens).max(usage.completion_tokens);
    if total_tokens > *current_turn_total_tokens {
        let delta = total_tokens - *current_turn_total_tokens;
        *thread_tokens = thread_tokens.saturating_add(u64::from(delta));
        *current_turn_total_tokens = total_tokens;
    }
    if total_tokens > 0 {
        *context_window_tokens = u64::from(total_tokens);
    }
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

fn top_layout_constraints() -> [Constraint; 2] {
    [Constraint::Length(1), Constraint::Min(5)]
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

fn roadmap_slash_path(plan: &str) -> String {
    if plan.starts_with("roadmap/") {
        plan.to_string()
    } else if plan.ends_with(".md") {
        format!("roadmap/{plan}")
    } else {
        format!("roadmap/{plan}.md")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use ratatui::{Terminal, backend::TestBackend};
    use roder_api::teams::{TeamMemberDescriptor, TeamMemberRole, TeamMemberStatus};
    use roder_app_server::AppServer;
    use roder_core::Runtime;
    use roder_protocol::{
        Item, ProviderDescriptor, ProvidersListResult, SpeechProvidersListResult, Thread,
        ThreadStatus, Turn,
    };

    fn test_app() -> TuiApp {
        let theme = Theme::for_dark_background(true);
        let server = Arc::new(AppServer::new(Arc::new(
            Runtime::fake().expect("fake runtime"),
        )));
        TuiApp {
            client: LocalAppClient::new(server.clone()),
            thread_id: "thread-test".to_string(),
            thread_title: None,
            thread_message_count: 0,
            active_turn_id: None,
            active_turn_timer: TurnTimer::default(),
            current_turn_input_tokens: 0,
            current_turn_output_tokens: 0,
            current_turn_reasoning_tokens: None,
            current_turn_total_tokens: 0,
            thread_tokens: 0,
            context_window_tokens: 0,
            provider: "mock".to_string(),
            model: "mock".to_string(),
            model_context_window: None,
            context_counter_hovered: false,
            last_frame_width: 100,
            selectable_lines: Vec::new(),
            mouse_selection: None,
            copied_helper: None,
            reasoning_effort: "medium".to_string(),
            composer: composer_textarea(theme),
            timeline: TimelineState::default(),
            team_ui: TeamUiState::default(),
            team_timelines: HashMap::new(),
            plan_panel: PlanPanelState::default(),
            tool_names: HashMap::new(),
            exec_session_tools: HashMap::new(),
            stdin_tool_sessions: HashMap::new(),
            hidden_stdin_tools: HashSet::new(),
            last_plan_counter_area: None,
            events: Vec::new(),
            animation_frame: 0,
            show_event_log: false,
            show_provider_popup: false,
            show_shortcuts_dialog: false,
            provider_popup_screen: ProviderPopupScreen::Main,
            provider_choices: Vec::new(),
            model_options: Vec::new(),
            pending_reasoning_model: None,
            pending_api_key_provider: None,
            provider_menu_items: Vec::new(),
            provider_menu_filter: String::new(),
            provider_state: ListState::default(),
            working_spinner: WorkingSpinner::Dots,
            scroll_settings: ScrollSettings::default(),
            timeline_settings: TimelineSettings::default(),
            web_search_mode: HostedWebSearchMode::Cached,
            search_index_enabled: true,
            command_shell: "bash".to_string(),
            command_shell_options: vec!["zsh".to_string(), "bash".to_string()],
            file_backed_dynamic_context: true,
            confirm_dialog: None,
            tool_detail_modal: None,
            plugin_browser: None,
            remote_panel: RemotePanelController::with_listen(
                server,
                "ws://127.0.0.1:0".to_string(),
                Some("/tmp/gode".to_string()),
            ),
            roadmap_mode: None,
            image_attachments: Vec::new(),
            queued_prompts: PromptQueue::default(),
            last_user_prompt: None,
            command_catalog: built_in_command_catalog(),
            slash_command_selection: 0,
            voice: VoiceState::default(),
            policy_mode: PolicyMode::Default,
            pending_plan_exit: None,
            current_goal: None,
            compaction_active: false,
            theme,
            active_theme_id: None,
            theme_preview_baseline: None,
        }
    }

    #[tokio::test]
    async fn remote_slash_command_starts_stops_and_displays_qr() {
        let mut app = test_app();

        app.run_remote_slash_command("start").await;
        assert!(app.remote_panel.is_running());
        let rendered = rendered_timeline_text(&mut app);
        assert!(rendered.contains("Remote app-server: running"));
        assert!(rendered.contains("QR:"));
        assert!(rendered.contains("roder://connect"));

        app.run_remote_slash_command("stop").await;
        assert!(!app.remote_panel.is_running());
        let rendered = rendered_timeline_text(&mut app);
        assert!(rendered.contains("Remote app-server: stopped"));
    }

    fn rendered_timeline_text(app: &mut TuiApp) -> String {
        let render = app.timeline.render(app.theme, Rect::new(0, 0, 100, 200));
        rendered_text_rows(&render.text, 100, 200, render.text_scroll)
            .into_iter()
            .map(|row| row.text)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn resumed_task_ledger_tool_result_restores_plan_panel() {
        let mut app = test_app();
        let thread = Thread {
            id: "thread-ledger".to_string(),
            preview: String::new(),
            model_provider: "mock".to_string(),
            created_at: 0,
            updated_at: 0,
            status: ThreadStatus {
                kind: "idle".to_string(),
                active_turn_id: None,
                active_flags: Vec::new(),
            },
            cwd: "/tmp".to_string(),
            name: None,
            usage: None,
            turns: Some(vec![Turn {
                id: "turn-ledger".to_string(),
                items: vec![Item {
                    id: "tool-ledger".to_string(),
                    kind: "toolMessage".to_string(),
                    text: Some(
                        "Task ledger: 1/2 completed\n- completed: Inspect [inspect]\n- in_progress: Verify [verify]"
                            .to_string(),
                    ),
                    status: Some("completed".to_string()),
                    phase: None,
                    tool_name: Some("task_ledger.update".to_string()),
                    tool_call_id: Some("tool-ledger".to_string()),
                    payload: None,
                }],
                items_view: "all".to_string(),
                status: "completed".to_string(),
                error: None,
                started_at: None,
                completed_at: None,
                duration_ms: None,
                usage: None,
            }]),
        };

        app.apply_thread(thread);

        assert!(app.plan_panel.is_visible());
        assert_eq!(app.plan_panel.len(), 2);
        assert_eq!(app.plan_panel.completed_count(), 1);
    }

    #[test]
    fn apply_thread_uses_protocol_active_turn_status() {
        let mut app = test_app();
        let running = Thread {
            id: "thread-running".to_string(),
            preview: String::new(),
            model_provider: "mock".to_string(),
            created_at: 0,
            updated_at: 0,
            status: ThreadStatus {
                kind: "running".to_string(),
                active_turn_id: Some("turn-live".to_string()),
                active_flags: Vec::new(),
            },
            cwd: "/tmp".to_string(),
            name: None,
            turns: None,
        };

        app.apply_thread(running);

        assert_eq!(app.active_turn_id.as_deref(), Some("turn-live"));

        let idle = Thread {
            id: "thread-idle".to_string(),
            preview: String::new(),
            model_provider: "mock".to_string(),
            created_at: 0,
            updated_at: 0,
            status: ThreadStatus {
                kind: "idle".to_string(),
                active_turn_id: None,
                active_flags: Vec::new(),
            },
            cwd: "/tmp".to_string(),
            name: None,
            turns: None,
        };

        app.apply_thread(idle);

        assert_eq!(app.active_turn_id, None);
    }

    #[test]
    fn team_focus_swaps_visible_timeline_state() {
        let mut app = test_app();
        app.team_ui.set_team("team-1".to_string(), test_members());
        app.timeline.push_system("lead timeline");

        app.cycle_team_focus(true);
        assert!(app.team_timelines.contains_key("lead"));
        app.timeline.push_system("builder timeline");

        app.cycle_team_focus(false);
        let render = app.timeline.render(app.theme, Rect::new(0, 0, 80, 8));
        let rows = rendered_text_rows(&render.text, 80, 8, render.text_scroll);
        assert!(rows.iter().any(|row| row.text.contains("lead timeline")));
        assert!(!rows.iter().any(|row| row.text.contains("builder timeline")));
    }

    #[test]
    fn team_thread_routing_targets_focused_member_thread() {
        let mut app = test_app();
        app.team_ui.set_team("team-1".to_string(), test_members());

        assert_eq!(app.focused_thread_id(), "thread-lead");
        app.cycle_team_focus(true);
        assert_eq!(app.focused_thread_id(), "thread-builder");
    }

    #[test]
    fn roadmap_mode_is_visible_in_footer() {
        let mut app = test_app();

        app.enter_roadmap_mode(Some("roadmap/20-roadmapping-mode.md".to_string()));

        let mut buffer = Buffer::empty(Rect::new(0, 0, 100, 1));
        app.footer(100).render(buffer.area, &mut buffer);
        let footer = buffer_row_cells(&buffer, 0)
            .iter()
            .map(|cell| cell.symbol.as_str())
            .collect::<String>();
        assert!(footer.contains("roadmap:20-roadmapping-mode.md"));
    }

    #[test]
    fn voice_mode_is_visible_in_empty_composer_footer() {
        let mut app = test_app();
        app.voice.enable_for_test();

        let mut buffer = Buffer::empty(Rect::new(0, 0, 100, 1));
        app.footer(100).render(buffer.area, &mut buffer);
        let footer = buffer_row_cells(&buffer, 0)
            .iter()
            .map(|cell| cell.symbol.as_str())
            .collect::<String>();

        assert!(footer.contains("hold Space to speak"));
    }

    #[test]
    fn footer_omits_policy_mode_label() {
        let mut app = test_app();
        app.policy_mode = PolicyMode::Bypass;

        let mut buffer = Buffer::empty(Rect::new(0, 0, 100, 1));
        app.footer(100).render(buffer.area, &mut buffer);
        let footer = buffer_row_cells(&buffer, 0)
            .iter()
            .map(|cell| cell.symbol.as_str())
            .collect::<String>();

        assert!(footer.contains("ready"));
        assert!(!footer.contains("mode:"));
    }

    #[test]
    fn roadmap_slash_command_enters_mode_with_plan() {
        let mut app = test_app();

        app.run_roadmap_slash_command("20-roadmapping-mode.md");

        assert_eq!(
            app.roadmap_mode
                .as_ref()
                .and_then(|state| state.selected_plan.as_deref()),
            Some("roadmap/20-roadmapping-mode.md")
        );
    }

    #[tokio::test]
    async fn provider_menu_roadmap_entry_enters_mode() {
        let mut app = test_app();
        app.show_provider_popup = true;
        app.provider_menu_items = main_provider_menu_items(&[]);
        app.provider_state.select(Some(3));

        app.select_current_provider_menu_item().await;

        assert!(!app.show_provider_popup);
        assert!(app.roadmap_mode.is_some());
    }

    #[test]
    fn roadmap_prompt_submission_adds_context() {
        let mut app = test_app();
        app.enter_roadmap_mode(Some("roadmap/20-roadmapping-mode.md".to_string()));
        app.composer.insert_str("continue the selected task");

        let pending = app.take_prepared_prompt().unwrap();

        assert_eq!(pending.display, "continue the selected task");
        assert!(pending.message.contains("Roadmapping mode is active"));
        assert!(pending.message.contains("Selected roadmap:"));
        assert!(pending.message.contains("continue the selected task"));
    }

    #[test]
    fn roadmap_attached_thread_becomes_prompt_target() {
        let mut app = test_app();
        app.enter_roadmap_mode(Some("roadmap/20-roadmapping-mode.md".to_string()));
        app.roadmap_mode
            .as_mut()
            .unwrap()
            .attach_thread("thread-roadmap");

        assert_eq!(app.focused_thread_id(), "thread-roadmap");
    }

    #[test]
    fn roadmap_document_keeps_activity_as_secondary_evidence() {
        let mut app = test_app();
        app.enter_roadmap_mode(Some("roadmap/20-roadmapping-mode.md".to_string()));
        app.timeline.push_system("worker evidence");

        let mut buffer = Buffer::empty(Rect::new(0, 0, 100, 80));
        app.roadmap_document(buffer.area)
            .render(buffer.area, &mut buffer);
        let rows = (0..buffer.area.height)
            .map(|row| {
                buffer_row_cells(&buffer, row)
                    .iter()
                    .map(|cell| cell.symbol.as_str())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rows.contains("Roadmap"));
        assert!(rows.contains("Activity Evidence"));
        assert!(rows.contains("worker evidence"));
    }

    #[test]
    fn roadmap_mode_uses_custom_workspace_instead_of_chat_frame() {
        let mut app = test_app();
        app.enter_roadmap_mode(Some("roadmap/20-roadmapping-mode.md".to_string()));

        let backend = TestBackend::new(120, 36);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.render(f)).unwrap();
        let buffer = terminal.backend().buffer();
        let rows = (0..buffer.area.height)
            .map(|row| {
                buffer_row_cells(buffer, row)
                    .iter()
                    .map(|cell| cell.symbol.as_str())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rows.contains("Roadmap Manager"));
        assert!(rows.contains("Task Queue"));
        assert!(rows.contains("Agent Lanes"));
        assert!(rows.contains("Validation"));
        assert!(!rows.contains("Send"));
    }

    #[tokio::test]
    async fn roadmap_workspace_keys_manage_tasks_and_leave_mode() {
        let mut app = test_app();
        let workspace = temp_roadmap_workspace();
        std::fs::write(
            workspace.join("roadmap/20-roadmapping-mode.md"),
            "# Roadmapping Mode Implementation Plan\n\n**Goal:** Add roadmapping mode.\n**Architecture:** Roadmap documents are first-class state.\n**Tech Stack:** Rust.\n\n## Owned Paths\n\n- Modify: `crates/roder-tui/src/roadmap.rs`\n\n## Tasks\n\n- [ ] Add roadmap tests\n- [ ] Wire roadmap keys\n\nRun:\n\n```sh\ncargo test -p roder-tui roadmap\n```\n\nAcceptance:\n- Roadmap mode renders.\n\n## Phase Acceptance\n\n- [ ] TUI works.\n",
        )
        .unwrap();
        std::fs::write(
            workspace.join("roadmap/21-second-plan.md"),
            "# Second Plan\n\n**Goal:** Exercise plan focus.\n**Architecture:** Roadmap documents are first-class state.\n**Tech Stack:** Rust.\n\n## Owned Paths\n\n- Modify: `crates/roder-tui/src/app.rs`\n\n## Tasks\n\n- [ ] Second plan task\n\nRun:\n\n```sh\ncargo test -p roder-tui roadmap\n```\n\nAcceptance:\n- Plan navigation works.\n\n## Phase Acceptance\n\n- [ ] TUI works.\n",
        )
        .unwrap();
        app.roadmap_mode = Some(
            RoadmapModeState::load(&workspace, Some("20-roadmapping-mode.md".to_string())).unwrap(),
        );

        assert_eq!(
            app.roadmap_mode
                .as_ref()
                .and_then(|state| state.focused_task_id.as_deref()),
            Some("task-add-roadmap-tests")
        );
        assert_eq!(
            app.roadmap_mode.as_ref().map(|state| state.focused_pane),
            Some(crate::roadmap::RoadmapPaneFocus::Tasks)
        );

        assert!(
            app.handle_roadmap_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
                .await
        );
        assert_eq!(
            app.roadmap_mode.as_ref().map(|state| state.focused_pane),
            Some(crate::roadmap::RoadmapPaneFocus::TaskDetail)
        );

        assert!(
            app.handle_roadmap_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT))
                .await
        );
        assert_eq!(
            app.roadmap_mode.as_ref().map(|state| state.focused_pane),
            Some(crate::roadmap::RoadmapPaneFocus::Tasks)
        );

        app.roadmap_mode.as_mut().unwrap().focused_pane = crate::roadmap::RoadmapPaneFocus::Plans;
        assert!(
            app.handle_roadmap_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
                .await
        );
        assert_eq!(
            app.roadmap_mode
                .as_ref()
                .and_then(|state| state.selected_plan.as_deref()),
            Some("roadmap/21-second-plan.md")
        );
        assert!(
            app.handle_roadmap_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE))
                .await
        );
        assert_eq!(
            app.roadmap_mode
                .as_ref()
                .and_then(|state| state.selected_plan.as_deref()),
            Some("roadmap/20-roadmapping-mode.md")
        );

        app.roadmap_mode.as_mut().unwrap().focused_pane = crate::roadmap::RoadmapPaneFocus::Tasks;
        assert!(
            app.handle_roadmap_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
                .await
        );
        assert_eq!(
            app.roadmap_mode
                .as_ref()
                .and_then(|state| state.focused_task_id.as_deref()),
            Some("task-wire-roadmap-keys")
        );

        app.roadmap_mode.as_mut().unwrap().attach_thread("thread-a");
        app.roadmap_mode.as_mut().unwrap().attach_thread("thread-b");
        app.roadmap_mode.as_mut().unwrap().focused_pane = crate::roadmap::RoadmapPaneFocus::Agents;
        assert!(
            app.handle_roadmap_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE))
                .await
        );
        assert_eq!(
            app.roadmap_mode
                .as_ref()
                .and_then(|state| state.selected_thread_id.as_deref()),
            Some("thread-a")
        );
        assert!(
            app.handle_roadmap_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
                .await
        );
        assert_eq!(
            app.roadmap_mode
                .as_ref()
                .and_then(|state| state.selected_thread_id.as_deref()),
            Some("thread-b")
        );

        app.roadmap_mode.as_mut().unwrap().focused_pane =
            crate::roadmap::RoadmapPaneFocus::Activity;
        assert!(
            app.handle_roadmap_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
                .await
        );
        assert_eq!(
            app.roadmap_mode.as_ref().map(|state| state.activity_scroll),
            Some(1)
        );
        assert!(
            app.handle_roadmap_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE))
                .await
        );
        assert_eq!(
            app.roadmap_mode.as_ref().map(|state| state.activity_scroll),
            Some(0)
        );

        assert!(
            app.handle_roadmap_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE))
                .await
        );
        assert!(
            app.roadmap_mode
                .as_ref()
                .is_some_and(|state| state.validation_diagnostics.is_empty())
        );

        assert!(
            app.handle_roadmap_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
                .await
        );
        assert!(app.roadmap_mode.is_none(), "events: {:?}", app.events);
    }

    #[tokio::test]
    async fn roadmap_worker_enter_loads_worker_chat_view() {
        let mut app = test_app();
        let worker = start_test_thread(&app).await;
        app.roadmap_mode = Some(RoadmapModeState::new(Some(
            "roadmap/20-roadmapping-mode.md".to_string(),
        )));
        let roadmap = app.roadmap_mode.as_mut().unwrap();
        roadmap.focused_pane = crate::roadmap::RoadmapPaneFocus::Agents;
        roadmap.attach_thread(worker.thread.id.clone());

        assert!(
            app.handle_roadmap_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
                .await
        );

        assert!(app.roadmap_mode.is_none());
        assert_eq!(app.thread_id, worker.thread.id);
        let render = app.timeline.render(app.theme, Rect::new(0, 0, 100, 8));
        let rows = rendered_text_rows(&render.text, 100, 8, render.text_scroll);
        assert!(
            rows.iter()
                .any(|row| row.text.contains("monitoring roadmap worker"))
        );
    }

    fn temp_roadmap_workspace() -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("roder-tui-roadmap-{unique}"));
        std::fs::create_dir_all(path.join("roadmap")).unwrap();
        path
    }

    async fn start_test_thread(app: &TuiApp) -> ThreadStartResult {
        let res = app
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("thread/start")),
                method: "thread/start".to_string(),
                params: Some(
                    serde_json::to_value(ThreadStartParams {
                        model: Some("mock".to_string()),
                        model_provider: Some("mock".to_string()),
                        cwd: std::env::current_dir().unwrap().display().to_string(),
                        ephemeral: false,
                    })
                    .unwrap(),
                ),
            })
            .await;
        decode_response::<ThreadStartResult>(res).unwrap()
    }

    #[test]
    fn interrupt_params_target_focused_team_member() {
        let mut app = test_app();
        app.team_ui.set_team("team-1".to_string(), test_members());
        app.cycle_team_focus(true);

        let params = app.interrupt_params("turn-team".to_string());

        assert_eq!(params.thread_id, "thread-builder");
        assert_eq!(params.turn_id.as_deref(), Some("turn-team"));
    }

    #[test]
    fn theme_primary_text_uses_terminal_default_for_contrast() {
        for dark in [true, false] {
            let theme = Theme::for_dark_background(dark);
            assert_eq!(theme.text, Color::Reset);
            assert_eq!(theme.text_strong, Color::Reset);
        }
    }

    #[test]
    fn commentary_theme_role_uses_white_on_dark_backgrounds() {
        assert_eq!(
            Theme::for_dark_background(true).commentary,
            Color::Indexed(15)
        );
        assert_eq!(
            Theme::for_dark_background(false).commentary,
            Color::Indexed(16)
        );
    }

    fn test_members() -> Vec<TeamMemberDescriptor> {
        vec![
            test_member("lead", TeamMemberRole::Lead, "Lead", "thread-lead"),
            test_member(
                "member-1",
                TeamMemberRole::Teammate,
                "Builder",
                "thread-builder",
            ),
        ]
    }

    fn test_member(
        id: &str,
        role: TeamMemberRole,
        name: &str,
        thread_id: &str,
    ) -> TeamMemberDescriptor {
        TeamMemberDescriptor {
            id: id.to_string(),
            role,
            name: name.to_string(),
            thread_id: thread_id.to_string(),
            current_turn_id: None,
            model_provider: None,
            model: None,
            policy_mode: PolicyMode::Default,
            status: TeamMemberStatus::Idle,
            pane_id: None,
        }
    }

    #[test]
    fn semantic_theme_roles_do_not_use_named_black_or_white() {
        for dark in [true, false] {
            let theme = Theme::for_dark_background(dark);
            let colors = [
                theme.text,
                theme.text_strong,
                theme.commentary,
                theme.muted,
                theme.subtle,
                theme.accent,
                theme.accent_soft,
                theme.tool,
                theme.tool_running,
                theme.diff_added,
                theme.diff_added_bg,
                theme.diff_removed,
                theme.diff_removed_bg,
                theme.diff_line_number,
                theme.shell,
                theme.error,
                theme.border,
                theme.mode_default,
                theme.mode_accept_all,
                theme.mode_plan,
                theme.mode_bypass,
                theme.dialog,
                theme.dialog_bg,
                theme.dialog_shadow,
                theme.dialog_key_bg,
                theme.selection_fg,
                theme.selection_bg,
            ];
            assert!(!colors.contains(&Color::White));
            assert!(!colors.contains(&Color::Black));
        }
    }

    #[test]
    fn keyboard_enhancements_request_all_keys_for_command_backspace() {
        assert!(
            keyboard_enhancement_flags()
                .contains(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
        assert!(
            keyboard_enhancement_flags()
                .contains(KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES)
        );
    }

    #[test]
    fn key_release_events_are_ignored() {
        let press = KeyEvent::new_with_kind(
            KeyCode::Char('p'),
            KeyModifiers::CONTROL,
            KeyEventKind::Press,
        );
        let repeat = KeyEvent::new_with_kind(
            KeyCode::Char('p'),
            KeyModifiers::CONTROL,
            KeyEventKind::Repeat,
        );
        let release = KeyEvent::new_with_kind(
            KeyCode::Char('p'),
            KeyModifiers::CONTROL,
            KeyEventKind::Release,
        );

        assert!(should_handle_key_event(press));
        assert!(should_handle_key_event(repeat));
        assert!(!should_handle_key_event(release));
    }

    #[test]
    fn timeline_mouse_click_takes_precedence_over_text_selection() {
        let mut app = test_app();
        app.timeline.record_tool_requested(
            "call_1".to_string(),
            ToolTimelineEntry::new("shell", r#"{"command":"printf hello"}"#),
        );
        app.timeline
            .record_tool_completed("call_1", false, Some("hello".to_string()));

        let transcript_area = Rect::new(0, 1, 100, 18);
        let _ = app.transcript(transcript_area);

        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 2,
            row: transcript_area.y,
            modifiers: KeyModifiers::empty(),
        });

        assert!(app.tool_detail_modal.is_some());
        assert!(app.mouse_selection.is_none());
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
    fn approval_confirm_dialog_allows_policy_mode_switch_key() {
        let approval = ConfirmDialogState::new(ConfirmDialog::ToolApproval {
            approval_id: "approval-1".to_string(),
            tool_name: "write_file".to_string(),
            reason: None,
        });
        let exit = ConfirmDialogState::new(ConfirmDialog::Exit);

        assert!(confirm_dialog_allows_policy_switch(&approval));
        assert!(!confirm_dialog_allows_policy_switch(&exit));
        assert!(is_policy_mode_switch_key(KeyEvent::new(
            KeyCode::BackTab,
            KeyModifiers::SHIFT
        )));
    }

    #[test]
    fn plan_panel_toggle_key_uses_ctrl_t() {
        assert!(is_plan_panel_toggle_key(KeyEvent::new(
            KeyCode::Char('t'),
            KeyModifiers::CONTROL
        )));
        assert!(!is_plan_panel_toggle_key(KeyEvent::new(
            KeyCode::Char('t'),
            KeyModifiers::NONE
        )));
        assert!(!is_plan_panel_toggle_key(KeyEvent::new(
            KeyCode::Char('l'),
            KeyModifiers::CONTROL
        )));
    }

    #[test]
    fn raw_tool_name_accepts_plain_tool_ids_only() {
        assert!(is_raw_tool_name("update_plan"));
        assert!(is_raw_tool_name("web-search.v1"));
        assert!(!is_raw_tool_name(""));
        assert!(!is_raw_tool_name("Update Plan awaiting approval"));
        assert!(!is_raw_tool_name("Grep path: crates query: thing"));
    }

    #[test]
    fn session_tool_helpers_accept_namespaced_tool_ids() {
        assert!(is_stdin_tool("write_stdin"));
        assert!(is_stdin_tool("functions.write_stdin"));
        assert!(is_exec_session_tool("exec_command"));
        assert!(is_exec_session_tool("functions.exec_command"));
        assert!(!is_exec_session_tool("shell"));
    }

    #[test]
    fn write_stdin_updates_original_exec_tool_in_timeline() {
        let mut app = test_app();
        app.record_tool_requested_with_id(
            "call_exec".to_string(),
            ToolTimelineEntry::new("exec_command", r#"{"cmd":"npm test"}"#),
        );
        app.record_tool_completed(
            "call_exec",
            false,
            Some(
                "Exit code: still running\nStatus: running\nWall time: 0.100 seconds\nSession id: 7\nOutput:\nstart"
                    .to_string(),
            ),
        );
        app.record_tool_requested_with_id(
            "call_stdin".to_string(),
            ToolTimelineEntry::new("write_stdin", r#"{"session_id":7}"#),
        );
        app.record_tool_completed(
            "call_stdin",
            false,
            Some(
                "Exit code: still running\nStatus: running\nWall time: 0.200 seconds\nSession id: 7\nOutput:\ndone"
                    .to_string(),
            ),
        );

        let rows = app
            .timeline
            .render(app.theme, Rect::new(0, 0, 100, 20))
            .text
            .lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert_eq!(rows.iter().filter(|row| row.contains("◆")).count(), 1);
        assert!(rows.iter().any(|row| row.contains("Exec Command")));
        assert!(rows.iter().any(|row| row.contains("done")));
        assert!(!rows.iter().any(|row| row.contains("Write Stdin")));
        assert!(app.exec_session_tools.contains_key(&7));

        app.record_tool_requested_with_id(
            "call_stdin_done".to_string(),
            ToolTimelineEntry::new("write_stdin", r#"{"session_id":7}"#),
        );
        app.record_tool_completed(
            "call_stdin_done",
            false,
            Some(
                "Exit code: 0\nStatus: completed\nWall time: 0.300 seconds\nSession id: 7\nOutput:\n(no output)"
                    .to_string(),
            ),
        );
        assert!(!app.exec_session_tools.contains_key(&7));
    }

    #[test]
    fn write_stdin_started_without_args_stays_hidden_and_updates_exec_row() {
        let mut app = test_app();
        app.record_tool_requested_with_id(
            "call_exec".to_string(),
            ToolTimelineEntry::new("exec_command", r#"{"cmd":"cargo test"}"#),
        );
        app.record_tool_completed(
            "call_exec",
            false,
            Some(
                "Exit code: still running\nStatus: running\nWall time: 0.100 seconds\nSession id: 9\nOutput:\ncompiling"
                    .to_string(),
            ),
        );

        app.record_tool_requested_with_id(
            "call_stdin".to_string(),
            ToolTimelineEntry::new("write_stdin", ""),
        );
        app.record_tool_delta("call_stdin", r#"{"session_id":9}"#);
        app.record_tool_requested_with_id(
            "call_stdin".to_string(),
            ToolTimelineEntry::new("write_stdin", r#"{"session_id":9}"#),
        );
        app.record_tool_completed(
            "call_stdin",
            false,
            Some(
                "Exit code: still running\nStatus: running\nWall time: 0.200 seconds\nSession id: 9\nOutput:\nfinished"
                    .to_string(),
            ),
        );

        let rows = app
            .timeline
            .render(app.theme, Rect::new(0, 0, 100, 20))
            .text
            .lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert_eq!(rows.iter().filter(|row| row.contains("◆")).count(), 1);
        assert!(rows.iter().any(|row| row.contains("Exec Command")));
        assert!(rows.iter().any(|row| row.contains("finished")));
        assert!(!rows.iter().any(|row| row.contains("Write Stdin")));
        assert!(!rows.iter().any(|row| row.contains("tool call_stdin")));
    }

    #[test]
    fn tool_approval_dialog_matches_only_matching_approval_id() {
        let approval = ConfirmDialogState::new(ConfirmDialog::ToolApproval {
            approval_id: "approval-1".to_string(),
            tool_name: "write_file".to_string(),
            reason: None,
        });
        let interrupt = ConfirmDialogState::new(ConfirmDialog::Interrupt);

        assert!(tool_approval_dialog_matches(&approval, "approval-1"));
        assert!(!tool_approval_dialog_matches(&approval, "approval-2"));
        assert!(!tool_approval_dialog_matches(&interrupt, "approval-1"));
    }

    #[test]
    fn dialog_menu_ctrl_j_and_ctrl_k_match_arrow_navigation() {
        assert!(is_dialog_menu_previous_key(KeyEvent::new(
            KeyCode::Up,
            KeyModifiers::NONE
        )));
        assert!(is_dialog_menu_previous_key(KeyEvent::new(
            KeyCode::Char('k'),
            KeyModifiers::CONTROL
        )));
        assert!(is_dialog_menu_next_key(KeyEvent::new(
            KeyCode::Down,
            KeyModifiers::NONE
        )));
        assert!(is_dialog_menu_next_key(KeyEvent::new(
            KeyCode::Char('j'),
            KeyModifiers::CONTROL
        )));
        assert!(!is_dialog_menu_next_key(KeyEvent::new(
            KeyCode::Char('j'),
            KeyModifiers::NONE
        )));
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
    fn context_window_counter_formats_compact_and_hovered_labels() {
        let usage = ContextWindowCounter {
            used_tokens: 15_800,
            max_tokens: 1_000_000,
            hovered: false,
        };
        assert_eq!(usage.label(), "│ 1.58% │");

        let hovered = ContextWindowCounter {
            hovered: true,
            ..usage
        };
        assert_eq!(hovered.label(), "│ 15K / 1.0M │");
    }

    #[test]
    fn usage_accounting_tracks_latest_context_window_separately_from_thread_total() {
        let mut input = 0;
        let mut output = 0;
        let mut turn_total = 0;
        let mut thread_total = 0;
        let mut context_total = 0;

        record_usage_counters(
            &mut input,
            &mut output,
            &mut turn_total,
            &mut thread_total,
            &mut context_total,
            TokenUsage::new(10_000, 1_000, 11_000),
        );
        record_usage_counters(
            &mut input,
            &mut output,
            &mut turn_total,
            &mut thread_total,
            &mut context_total,
            TokenUsage::new(10_500, 1_500, 12_000),
        );

        assert_eq!(input, 10_500);
        assert_eq!(output, 1_500);
        assert_eq!(turn_total, 12_000);
        assert_eq!(thread_total, 12_000);
        assert_eq!(context_total, 12_000);

        input = 0;
        output = 0;
        turn_total = 0;
        record_usage_counters(
            &mut input,
            &mut output,
            &mut turn_total,
            &mut thread_total,
            &mut context_total,
            TokenUsage::new(20_000, 2_000, 22_000),
        );

        assert_eq!(thread_total, 34_000);
        assert_eq!(context_total, 22_000);
    }

    #[test]
    fn working_status_label_reflects_server_side_compaction() {
        assert_eq!(working_status_label(false), "Working");
        assert_eq!(working_status_label(true), "Compacting context");
    }

    #[test]
    fn reasoning_token_count_is_extracted_from_provider_metadata() {
        let metadata = serde_json::json!({
            "usage": {
                "output_tokens_details": {
                    "reasoning_tokens": 2048
                }
            }
        });

        assert_eq!(
            reasoning_tokens_from_provider_metadata(&metadata),
            Some(2048)
        );
    }

    #[test]
    fn context_window_counter_hitbox_uses_hover_width_at_right_edge() {
        let usage = ContextWindowCounter {
            used_tokens: 15_800,
            max_tokens: 1_000_000,
            hovered: false,
        };
        assert!(usage.hit_test(120, 0, 108));
        assert!(usage.hit_test(120, 0, 119));
        assert!(!usage.hit_test(120, 0, 100));
        assert!(!usage.hit_test(120, 1, 119));
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
    fn message_lines_format_running_and_failed_tool_messages() {
        let theme = Theme::for_dark_background(true);
        let running = message_line("tool_running: Search query: rust", theme);
        let failed = message_line("tool_failed: Search query: rust", theme);

        assert_eq!(
            running
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>(),
            "◆ Search query: rust"
        );
        assert_eq!(
            failed
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>(),
            "◆ Search query: rust"
        );
        assert_eq!(failed.spans[0].style, theme.error());
        assert_eq!(failed.spans[1].style, theme.error());
    }

    #[test]
    fn working_elapsed_formats_like_status_text() {
        assert_eq!(format_working_elapsed(Duration::from_secs(9)), "9s");
        assert_eq!(format_working_elapsed(Duration::from_secs(175)), "2m 55s");
        assert_eq!(
            format_working_elapsed(Duration::from_secs(3_725)),
            "1h 2m 5s"
        );
    }

    #[test]
    fn spinner_frame_cycles_configured_frames() {
        assert_eq!(spinner_frame(WorkingSpinner::Line, 0), "-");
        assert_eq!(spinner_frame(WorkingSpinner::Line, 1), "\\");
        assert_eq!(spinner_frame(WorkingSpinner::Line, 4), "-");
    }

    #[test]
    fn padded_spinner_frames_keep_status_width_stable() {
        for spinner in WorkingSpinner::all() {
            let width = spinner_frame_width(*spinner);
            for frame in 0..16 {
                assert_eq!(padded_spinner_frame(*spinner, frame).chars().count(), width);
            }
        }
    }

    #[test]
    fn top_status_animation_interval_is_locked_to_calm_6fps() {
        assert_eq!(TOP_STATUS_ANIMATION_FPS, 6);
        assert_eq!(
            top_status_animation_interval(),
            Duration::from_nanos(166_666_666)
        );
    }

    #[test]
    fn top_status_animation_advances_at_most_one_frame_per_tick() {
        let start = Instant::now();
        let mut next_tick = start + top_status_animation_interval();
        let mut frame = 10;

        advance_top_status_animation(&mut frame, &mut next_tick, start);
        assert_eq!(frame, 10);
        assert_eq!(
            top_status_animation_poll_timeout(next_tick, start),
            top_status_animation_interval()
        );

        let delayed = start + Duration::from_secs(1);
        advance_top_status_animation(&mut frame, &mut next_tick, delayed);
        assert_eq!(frame, 11);
        assert_eq!(next_tick, delayed + top_status_animation_interval());
    }

    #[test]
    fn working_spinner_parses_config_ids() {
        assert_eq!(
            WorkingSpinner::from_config(Some("line")),
            WorkingSpinner::Line
        );
        assert_eq!(
            WorkingSpinner::from_config(Some("unknown")),
            WorkingSpinner::Dots
        );
        assert_eq!(WorkingSpinner::from_config(None), WorkingSpinner::Dots);
    }

    #[test]
    fn scroll_acceleration_is_enabled_by_default() {
        assert!(
            TuiUserConfig::default()
                .scroll_settings()
                .acceleration_enabled
        );
    }

    #[test]
    fn scroll_acceleration_can_be_disabled_in_config() {
        let config = TuiUserConfig {
            scroll_acceleration: Some(TuiScrollAccelerationConfig {
                enabled: Some(false),
            }),
            ..TuiUserConfig::default()
        };

        assert!(!config.scroll_settings().acceleration_enabled);
    }

    #[test]
    fn scroll_speed_config_sets_base_rows() {
        let config = TuiUserConfig {
            scroll_speed: Some(4.5),
            ..TuiUserConfig::default()
        };

        assert_eq!(config.scroll_settings().fixed_rows_per_tick, 4.5);
    }

    #[test]
    fn message_folding_is_disabled_by_default() {
        assert!(!TuiUserConfig::default().timeline_settings().message_folding);
    }

    #[test]
    fn message_folding_can_be_enabled_in_config() {
        let config = TuiUserConfig {
            timeline: Some(TuiTimelineConfig {
                message_folding: Some(true),
            }),
            ..TuiUserConfig::default()
        };

        assert!(config.timeline_settings().message_folding);
    }

    #[test]
    fn tui_config_path_targets_home_roder_config() {
        let rendered = tui_config_path().to_string_lossy().replace('\\', "/");
        assert!(rendered.ends_with("/.roder/config.toml"));
        assert!(!rendered.ends_with("/w/.roder/config.toml"));
    }

    #[test]
    fn top_layout_starts_with_header_and_transcript() {
        assert_eq!(
            top_layout_constraints(),
            [Constraint::Length(1), Constraint::Min(5)]
        );
    }

    #[test]
    fn event_log_height_only_allocates_space_when_toggled_on() {
        assert_eq!(event_log_height(false, 10), 0);
        assert_eq!(event_log_height(true, 0), 3);
        assert_eq!(event_log_height(true, 3), 5);
        assert_eq!(event_log_height(true, 100), 8);
    }

    #[test]
    fn copied_helper_expires_after_duration() {
        let start = Instant::now();
        let helper = CopiedHelper { shown_at: start };

        assert!(helper.visible(start + COPIED_HELPER_DURATION - Duration::from_millis(1)));
        assert!(!helper.visible(start + COPIED_HELPER_DURATION));
    }

    #[test]
    fn copied_helper_area_sits_above_prompt_right_edge() {
        let composer = Rect::new(0, 20, 100, 3);
        let area = copied_helper_area(composer).expect("helper should fit above composer");

        assert_eq!(area.y, 19);
        assert_eq!(area.height, 1);
        assert_eq!(area.width, copied_helper_width());
        assert_eq!(area.x + area.width, 98);
    }

    #[test]
    fn copied_helper_area_is_absent_on_top_row() {
        assert_eq!(copied_helper_area(Rect::new(0, 0, 100, 3)), None);
    }

    #[test]
    fn voice_helper_sits_inside_composer_top_right() {
        let composer = Rect::new(4, 5, 80, 3);
        let area = voice_helper_area(composer).expect("voice helper should fit");

        assert_eq!(area.y, composer.y);
        assert_eq!(area.height, 1);
        assert_eq!(area.width, voice_helper_width());
        assert!(area.x >= composer.x);
        assert!(area.x + area.width <= composer.x + composer.width);
    }

    #[test]
    fn rect_contains_uses_half_open_bounds() {
        let area = Rect::new(10, 20, 5, 3);

        assert!(rect_contains(area, 10, 20));
        assert!(rect_contains(area, 14, 22));
        assert!(!rect_contains(area, 15, 22));
        assert!(!rect_contains(area, 14, 23));
        assert!(!rect_contains(area, 9, 20));
        assert!(!rect_contains(area, 10, 19));
    }

    #[test]
    fn transcript_scroll_offset_follows_latest_lines() {
        assert_eq!(transcript_scroll_offset(3, 10), 0);
        assert_eq!(transcript_scroll_offset(10, 10), 0);
        assert_eq!(transcript_scroll_offset(14, 10), 4);
    }

    #[test]
    fn policy_mode_switcher_cycles_non_bypass_modes() {
        assert_eq!(next_policy_mode(PolicyMode::Default), PolicyMode::AcceptAll);
        assert_eq!(next_policy_mode(PolicyMode::AcceptAll), PolicyMode::Plan);
        assert_eq!(next_policy_mode(PolicyMode::Plan), PolicyMode::Default);
        assert_eq!(next_policy_mode(PolicyMode::Bypass), PolicyMode::Default);
    }

    #[test]
    fn policy_mode_labels_match_protocol_values() {
        assert_eq!(policy_mode_label(PolicyMode::Default), "default");
        assert_eq!(policy_mode_label(PolicyMode::AcceptAll), "accept_all");
        assert_eq!(policy_mode_label(PolicyMode::Plan), "plan");
        assert_eq!(policy_mode_label(PolicyMode::Bypass), "bypass");
    }

    #[test]
    fn pretty_policy_mode_labels_are_human_readable() {
        assert_eq!(pretty_policy_mode_label(PolicyMode::Default), "Default");
        assert_eq!(
            pretty_policy_mode_label(PolicyMode::AcceptAll),
            "Accept All"
        );
        assert_eq!(pretty_policy_mode_label(PolicyMode::Plan), "Plan");
        assert_eq!(pretty_policy_mode_label(PolicyMode::Bypass), "Bypass");
    }

    #[test]
    fn policy_mode_styles_are_distinct() {
        for dark in [true, false] {
            let theme = Theme::for_dark_background(dark);
            let colors = [
                theme.policy_mode(PolicyMode::Default).fg,
                theme.policy_mode(PolicyMode::AcceptAll).fg,
                theme.policy_mode(PolicyMode::Plan).fg,
                theme.policy_mode(PolicyMode::Bypass).fg,
            ];

            for (index, color) in colors.iter().enumerate() {
                assert!(
                    colors
                        .iter()
                        .enumerate()
                        .all(|(other_index, other)| index == other_index || color != other),
                    "{colors:?}"
                );
            }
        }
    }

    #[test]
    fn image_attachment_height_only_allocates_when_images_are_attached() {
        assert_eq!(image_attachment_height(0), 0);
        assert_eq!(image_attachment_height(1), 2);
        assert_eq!(image_attachment_height(3), 4);
        assert_eq!(image_attachment_height(10), 4);
    }

    #[test]
    fn queued_prompt_height_tracks_visible_rows_and_overflow() {
        assert_eq!(queued_prompt_height(0), 0);
        assert_eq!(queued_prompt_height(1), 2);
        assert_eq!(queued_prompt_height(3), 4);
        assert_eq!(queued_prompt_height(4), 5);
    }

    #[test]
    fn mouse_selection_extracts_text_across_rows() {
        let lines = vec![
            SelectableLine {
                row: 2,
                text: "hello world".to_string(),
            },
            SelectableLine {
                row: 3,
                text: "second line".to_string(),
            },
        ];
        let selection = MouseSelection {
            anchor: SelectionPoint { row: 2, column: 6 },
            cursor: SelectionPoint { row: 3, column: 5 },
            dragging: true,
        };

        assert_eq!(
            selected_text(&lines, &selection).as_deref(),
            Some("world\nsecond")
        );
    }

    #[test]
    fn mouse_selection_normalizes_reverse_drags() {
        let lines = vec![SelectableLine {
            row: 4,
            text: "abcdef".to_string(),
        }];
        let selection = MouseSelection {
            anchor: SelectionPoint { row: 4, column: 4 },
            cursor: SelectionPoint { row: 4, column: 1 },
            dragging: true,
        };

        assert_eq!(selected_text(&lines, &selection).as_deref(), Some("bcde"));
    }

    #[test]
    fn mouse_selection_uses_wrapped_visual_rows() {
        let text = Text::from(Line::raw("abc def ghi jkl mno"));
        let area = Rect::new(0, 5, 7, 4);

        let rows = selectable_lines_from_text(&text, area, 0);

        assert_eq!(
            rows,
            vec![
                SelectableLine {
                    row: 5,
                    text: "abc def".to_string()
                },
                SelectableLine {
                    row: 6,
                    text: "ghi jkl".to_string()
                },
                SelectableLine {
                    row: 7,
                    text: "mno".to_string()
                },
            ]
        );

        let selection = MouseSelection {
            anchor: SelectionPoint { row: 6, column: 4 },
            cursor: SelectionPoint { row: 6, column: 6 },
            dragging: true,
        };
        assert_eq!(selected_text(&rows, &selection).as_deref(), Some("jkl"));
    }

    #[test]
    fn mouse_selection_highlights_wrapped_visual_rows() {
        let text = Text::from(Line::raw("abc def ghi jkl mno"));
        let area = Rect::new(0, 5, 7, 4);
        let theme = Theme::for_dark_background(true);
        let selection = MouseSelection {
            anchor: SelectionPoint { row: 6, column: 4 },
            cursor: SelectionPoint { row: 6, column: 6 },
            dragging: true,
        };

        let highlighted = highlight_selection(text, area, 0, &selection, theme);
        let row = &highlighted.lines[1];

        assert_eq!(row.spans[4].style, theme.selected());
        assert_eq!(row.spans[5].style, theme.selected());
        assert_eq!(row.spans[6].style, theme.selected());
    }

    #[test]
    fn mouse_selection_scroll_uses_visual_rows() {
        let text = Text::from(Line::raw("abc def ghi jkl mno"));
        let area = Rect::new(0, 5, 7, 2);

        let rows = selectable_lines_from_text(&text, area, 2);

        assert_eq!(
            rows,
            vec![SelectableLine {
                row: 5,
                text: "mno".to_string()
            }]
        );
    }

    #[test]
    fn slash_command_menu_height_tracks_visible_matches() {
        let commands = built_in_command_catalog();
        let matches = commands.iter().collect::<Vec<_>>();

        assert_eq!(slash_command_menu_height(None::<&[CommandDescriptor]>), 0);
        assert_eq!(
            slash_command_menu_height(Some(&[] as &[CommandDescriptor])),
            0
        );
        assert_eq!(slash_command_menu_height(Some(&matches[..1])), 2);
        assert_eq!(
            slash_command_menu_height(Some(&matches)),
            1 + MAX_VISIBLE_SLASH_COMMANDS as u16
        );
    }

    #[test]
    fn slash_command_preview_height_is_collapsed_to_one_line() {
        let commands = built_in_command_catalog();
        assert_eq!(
            slash_command_preview_height(None::<&[CommandDescriptor]>),
            0
        );
        assert_eq!(
            slash_command_preview_height(Some(&[] as &[CommandDescriptor])),
            0
        );
        assert_eq!(slash_command_preview_height(Some(&commands)), 1);
    }

    #[test]
    fn image_paste_detects_absolute_and_escaped_image_paths() {
        #[cfg(windows)]
        let (paste, first, second) = (
            r"C:\tmp\first.png C:\tmp\second\ image.jpg",
            PathBuf::from(r"C:\tmp\first.png"),
            PathBuf::from(r"C:\tmp\second image.jpg"),
        );
        #[cfg(not(windows))]
        let (paste, first, second) = (
            "/tmp/first.png /tmp/second\\ image.jpg",
            PathBuf::from("/tmp/first.png"),
            PathBuf::from("/tmp/second image.jpg"),
        );

        let attachments = image_attachments_from_paste(paste).expect("expected image attachments");

        assert_eq!(attachments.len(), 2);
        assert_eq!(attachments[0].path, first);
        assert_eq!(attachments[1].path, second);
    }

    #[test]
    fn image_paste_detects_file_uris() {
        #[cfg(windows)]
        let (paste, path) = (
            "file:///C:/tmp/Screen%20Shot.webp",
            PathBuf::from("C:/tmp/Screen Shot.webp"),
        );
        #[cfg(not(windows))]
        let (paste, path) = (
            "file:///tmp/Screen%20Shot.webp",
            PathBuf::from("/tmp/Screen Shot.webp"),
        );

        let attachments = image_attachments_from_paste(paste).expect("expected image attachment");

        assert_eq!(attachments[0].path, path);
    }

    #[test]
    fn image_paste_ignores_mixed_text() {
        assert!(image_attachments_from_paste("look at /tmp/image.png please").is_none());
    }

    #[test]
    fn prompt_images_are_encoded_as_data_urls() {
        let path = std::env::temp_dir().join(format!(
            "roder-tui-image-attachment-{}.png",
            std::process::id()
        ));
        std::fs::write(&path, b"abc").unwrap();
        let attachments = vec![ImageAttachment::new(path.clone())];

        let images = image_inputs_from_attachments(&attachments).unwrap();
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].image_url, "data:image/png;base64,YWJj");
        assert_eq!(
            mime_type_for_image_path(&PathBuf::from("/tmp/diagram.webp")),
            "image/webp"
        );
        assert_eq!(
            transcript_message_with_image_attachments("", &attachments),
            format!(
                "attached image: {}",
                path.file_name().unwrap().to_string_lossy()
            )
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn pending_turn_input_preserves_images_as_input_items() {
        let input = pending_turn_input(
            "what do you see?".to_string(),
            vec![InputImage {
                image_url: "data:image/png;base64,YWJj".to_string(),
            }],
        );

        assert_eq!(input.len(), 2);
        assert_eq!(input[0].kind, "text");
        assert_eq!(input[0].text.as_deref(), Some("what do you see?"));
        assert_eq!(input[1].kind, "image");
        assert_eq!(
            input[1].image_url.as_deref(),
            Some("data:image/png;base64,YWJj")
        );
    }

    #[test]
    fn provider_options_include_provider_models() {
        let list = ProvidersListResult {
            active_provider: "mock".to_string(),
            active_model: "mock".to_string(),
            active_reasoning: "medium".to_string(),
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
                    context_window: Some(123_000),
                    default_reasoning: Some("medium".to_string()),
                    supported_reasoning: vec![ReasoningEffortDescriptor {
                        effort: "medium".to_string(),
                        description: "Balanced reasoning".to_string(),
                    }],
                }],
            }],
        };

        let options = provider_options_from_list(&list);
        assert_eq!(options.len(), 1);
        assert_eq!(options[0].provider_id, "mock");
        assert_eq!(options[0].model_id, "mock");
        assert_eq!(options[0].context_window, Some(123_000));
        assert_eq!(options[0].default_reasoning.as_deref(), Some("medium"));
        assert_eq!(options[0].reasoning_options.len(), 1);
    }

    #[test]
    fn provider_menu_starts_with_providers_submenu() {
        let items = main_provider_menu_items(&[]);
        assert!(matches!(items.first(), Some(ProviderMenuItem::Models)));
        assert!(matches!(items.get(1), Some(ProviderMenuItem::Providers)));
        assert!(matches!(items.get(2), Some(ProviderMenuItem::Settings)));
        assert!(matches!(items.get(3), Some(ProviderMenuItem::RoadmapMode)));
        assert!(matches!(
            items.get(4),
            Some(ProviderMenuItem::RunnerSettings)
        ));
        assert!(matches!(
            items.get(5),
            Some(ProviderMenuItem::ResumeThreads)
        ));
        assert!(matches!(
            items.get(6),
            Some(ProviderMenuItem::WebSearchSettings)
        ));
        assert!(matches!(
            items.get(7),
            Some(ProviderMenuItem::SpinnerSettings)
        ));
        assert!(matches!(
            items.get(8),
            Some(ProviderMenuItem::ThemesSettings)
        ));
        assert!(matches!(
            items.get(9),
            Some(ProviderMenuItem::MarketplacesSettings)
        ));
    }

    #[test]
    fn runner_menu_items_select_runner_providers() {
        let items = runner_menu_items(&RunnersListResult {
            active: Some(roder_protocol::RunnerStatus {
                destination_id: "unix-local".to_string(),
                provider_id: "unix-local".to_string(),
                state: "active".to_string(),
                session_id: None,
            }),
            providers: vec![roder_protocol::RunnerProviderDescriptor {
                provider_id: "unix-local".to_string(),
                capabilities: roder_api::remote_runner::RunnerCapabilities {
                    command_exec: true,
                    file_read: true,
                    file_write: true,
                    port_preview: false,
                    snapshots: false,
                    cancellation: true,
                    artifact_export: false,
                    mounts: Default::default(),
                },
            }],
        });

        assert!(matches!(
            &items[0],
            ProviderMenuItem::Runner {
                destination_id,
                provider_id,
                label
            } if destination_id == "unix-local"
                && provider_id == "unix-local"
                && label.contains("(active)")
                && label.contains("commands")
                && label.contains("cancel")
        ));
        assert!(matches!(items.last(), Some(ProviderMenuItem::Back)));
    }

    #[test]
    fn settings_menu_labels_default_modes_for_users() {
        assert_eq!(ProviderMenuItem::Settings.label(), "Settings");
        assert_eq!(
            ProviderMenuItem::DefaultMode(PolicyMode::AcceptAll).label(),
            "Default mode: Accept edits"
        );
        assert_eq!(
            ProviderMenuItem::MessageFoldingToggle(false).label(),
            "Fold long messages: off"
        );
        assert_eq!(
            ProviderMenuItem::SearchIndexToggle(true).label(),
            "Instant regex search: on"
        );
        assert_eq!(
            ProviderMenuItem::ShellSettings("bash".to_string()).label(),
            "Shell command shell: bash"
        );
        assert_eq!(ProviderMenuItem::VoiceModelSettings.label(), "Voice model");
        assert_eq!(
            ProviderMenuItem::FileBackedDynamicContextToggle(true).label(),
            "File-backed dynamic context: on"
        );
        assert_eq!(settings_policy_mode_label(PolicyMode::Default), "Default");
    }

    #[test]
    fn settings_menu_includes_toggles_before_back() {
        let items = settings_menu_items(TimelineSettings::default(), true, "bash", true);

        assert!(matches!(
            items.get(4),
            Some(ProviderMenuItem::SearchIndexToggle(true))
        ));
        assert!(matches!(
            items.get(5),
            Some(ProviderMenuItem::ShellSettings(shell)) if shell == "bash"
        ));
        assert!(matches!(
            items.get(6),
            Some(ProviderMenuItem::VoiceModelSettings)
        ));
        assert!(matches!(
            items.get(7),
            Some(ProviderMenuItem::FileBackedDynamicContextToggle(true))
        ));
        assert!(matches!(
            items.get(8),
            Some(ProviderMenuItem::MessageFoldingToggle(false))
        ));
        assert!(matches!(items.get(9), Some(ProviderMenuItem::Back)));
    }

    #[test]
    fn voice_model_menu_groups_speech_models_by_provider() {
        let items = voice_model_menu_items(&SpeechProvidersListResult {
            providers: vec![roder_protocol::SpeechProviderDescriptor {
                id: "openai-speech".to_string(),
                name: "OpenAI".to_string(),
                description: None,
                auth_type: ProviderAuthType::ApiKey,
                auth_label: Some("OPENAI_API_KEY".to_string()),
                authenticated: true,
                auth_detail: None,
                recommended: true,
                sort_order: 0,
                capabilities: roder_api::speech::SpeechCapabilities::default(),
                models: vec![roder_api::speech::SpeechModelDescriptor {
                    id: "gpt-4o-mini-transcribe".to_string(),
                    name: "GPT-4o Mini Transcribe".to_string(),
                    description: None,
                    capabilities: roder_api::speech::SpeechCapabilities::default(),
                }],
            }],
        });

        assert!(matches!(
            items.first(),
            Some(ProviderMenuItem::Section(label)) if label == "OpenAI"
        ));
        assert!(matches!(
            &items[1],
            ProviderMenuItem::VoiceModel(choice)
                if choice.provider_id == "openai-speech"
                    && choice.model_id == "gpt-4o-mini-transcribe"
                    && choice.label.contains("GPT-4o Mini Transcribe")
        ));
        assert!(matches!(items.last(), Some(ProviderMenuItem::Back)));
    }

    #[test]
    fn themes_submenu_label_matches_theme_id() {
        let item = ProviderMenuItem::Theme("midnight".to_string());
        assert_eq!(item.label(), "midnight");
        assert_eq!(ProviderMenuItem::ThemesSettings.label(), "Themes");
    }

    #[test]
    fn web_search_settings_menu_lists_hosted_modes() {
        let items = [
            HostedWebSearchMode::Cached,
            HostedWebSearchMode::Live,
            HostedWebSearchMode::Disabled,
        ]
        .into_iter()
        .map(ProviderMenuItem::WebSearchMode)
        .chain(std::iter::once(ProviderMenuItem::Back))
        .collect::<Vec<_>>();

        assert_eq!(items[0].label(), "Cached hosted");
        assert_eq!(items[1].label(), "Live hosted");
        assert_eq!(items[2].label(), "Disabled");
        assert!(matches!(items[3], ProviderMenuItem::Back));
    }

    #[test]
    fn provider_choices_live_under_providers_submenu() {
        let provider = ProviderChoice {
            provider_id: "codex".to_string(),
            name: "Codex".to_string(),
            description: Some("ChatGPT account provider".to_string()),
            auth_type: ProviderAuthType::OAuth,
            authenticated: false,
            auth_detail: None,
            default_model: Some("gpt-5.5".to_string()),
            recommended: true,
        };

        let main = main_provider_menu_items(std::slice::from_ref(&provider));
        assert!(
            !main
                .iter()
                .any(|item| matches!(item, ProviderMenuItem::Provider(_)))
        );

        let submenu = providers_menu_items(&[provider]);
        assert!(matches!(
            submenu.first(),
            Some(ProviderMenuItem::Provider(_))
        ));
        assert!(matches!(submenu.last(), Some(ProviderMenuItem::Back)));
    }

    #[test]
    fn model_menu_groups_models_under_provider_sections() {
        let providers = vec![
            ProviderChoice {
                provider_id: "opencode".to_string(),
                name: "OpenCode Zen".to_string(),
                description: None,
                auth_type: ProviderAuthType::ApiKey,
                authenticated: true,
                auth_detail: None,
                default_model: Some("big-pickle".to_string()),
                recommended: false,
            },
            ProviderChoice {
                provider_id: "opencode-go".to_string(),
                name: "OpenCode Go".to_string(),
                description: None,
                auth_type: ProviderAuthType::ApiKey,
                authenticated: true,
                auth_detail: None,
                default_model: Some("qwen3.6-plus".to_string()),
                recommended: false,
            },
            ProviderChoice {
                provider_id: "anthropic".to_string(),
                name: "Anthropic".to_string(),
                description: None,
                auth_type: ProviderAuthType::ApiKey,
                authenticated: true,
                auth_detail: None,
                default_model: Some("claude-opus-4.1".to_string()),
                recommended: false,
            },
        ];
        let models = vec![
            ProviderOption {
                provider_id: "opencode".to_string(),
                model_id: "big-pickle".to_string(),
                label: "opencode/big-pickle (Big Pickle)".to_string(),
                context_window: None,
                default_reasoning: None,
                reasoning_options: Vec::new(),
            },
            ProviderOption {
                provider_id: "opencode-go".to_string(),
                model_id: "qwen3.6-plus".to_string(),
                label: "opencode-go/qwen3.6-plus (Qwen3.6 Plus)".to_string(),
                context_window: None,
                default_reasoning: None,
                reasoning_options: Vec::new(),
            },
            ProviderOption {
                provider_id: "anthropic".to_string(),
                model_id: "claude-opus-4.1".to_string(),
                label: "anthropic/claude-opus-4.1 (Claude Opus 4.1)".to_string(),
                context_window: None,
                default_reasoning: None,
                reasoning_options: Vec::new(),
            },
        ];

        let items = models_menu_items(&models, &providers);

        assert!(matches!(
            &items[0],
            ProviderMenuItem::Section(label) if label == "OpenCode Zen"
        ));
        assert!(matches!(
            &items[1],
            ProviderMenuItem::Model(option)
                if option.provider_id == "opencode"
                    && option.model_id == "big-pickle"
        ));
        assert!(matches!(
            &items[2],
            ProviderMenuItem::Section(label) if label == "OpenCode Go"
        ));
        assert!(matches!(
            &items[3],
            ProviderMenuItem::Model(option)
                if option.provider_id == "opencode-go"
                    && option.model_id == "qwen3.6-plus"
        ));
        assert!(matches!(
            &items[4],
            ProviderMenuItem::Section(label) if label == "Anthropic"
        ));
        assert!(matches!(
            &items[5],
            ProviderMenuItem::Model(option)
                if option.provider_id == "anthropic"
                    && option.model_id == "claude-opus-4.1"
        ));
        assert!(matches!(items.last(), Some(ProviderMenuItem::Back)));
    }

    #[test]
    fn provider_menu_filter_matches_labels_case_insensitively() {
        let items = providers_menu_items(&[ProviderChoice {
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
            ProviderMenuItem::Section("Codex".to_string()),
            ProviderMenuItem::Model(ProviderOption {
                provider_id: "codex".to_string(),
                model_id: "gpt-5.5".to_string(),
                label: "codex/gpt-5.5 (GPT-5.5)".to_string(),
                context_window: Some(1_000_000),
                default_reasoning: Some("medium".to_string()),
                reasoning_options: Vec::new(),
            }),
        ];
        let filtered = filter_provider_menu_items(&items, "5.5");
        assert_eq!(filtered.len(), 1);
        assert!(matches!(filtered[0], ProviderMenuItem::Model(_)));
    }

    #[test]
    fn provider_menu_includes_cursor_composer_model() {
        let providers = vec![ProviderChoice {
            provider_id: "cursor".to_string(),
            name: "Cursor".to_string(),
            description: Some("Cursor Composer via direct AgentService API".to_string()),
            auth_type: ProviderAuthType::ApiKey,
            authenticated: true,
            auth_detail: None,
            default_model: Some("composer-2.5".to_string()),
            recommended: true,
        }];
        let models = vec![ProviderOption {
            provider_id: "cursor".to_string(),
            model_id: "composer-2.5".to_string(),
            label: "cursor/composer-2.5 (Composer 2.5)".to_string(),
            context_window: Some(200_000),
            default_reasoning: None,
            reasoning_options: Vec::new(),
        }];

        let items = models_menu_items(&models, &providers);

        assert!(matches!(
            &items[0],
            ProviderMenuItem::Section(label) if label == "Cursor"
        ));
        assert!(matches!(
            &items[1],
            ProviderMenuItem::Model(option)
                if option.provider_id == "cursor"
                    && option.model_id == "composer-2.5"
        ));
    }

    #[test]
    fn provider_menu_filter_does_not_return_section_headers() {
        let items = vec![
            ProviderMenuItem::Section("OpenCode".to_string()),
            ProviderMenuItem::Model(ProviderOption {
                provider_id: "opencode-go".to_string(),
                model_id: "qwen3.6-plus".to_string(),
                label: "opencode-go/qwen3.6-plus (Qwen3.6 Plus)".to_string(),
                context_window: None,
                default_reasoning: None,
                reasoning_options: Vec::new(),
            }),
        ];

        let filtered = filter_provider_menu_items(&items, "opencode");

        assert_eq!(filtered.len(), 1);
        assert!(matches!(filtered[0], ProviderMenuItem::Model(_)));
    }

    #[test]
    fn provider_choice_label_prompts_for_missing_api_key() {
        let provider = ProviderChoice {
            provider_id: "opencode".to_string(),
            name: "OpenCode".to_string(),
            description: None,
            auth_type: ProviderAuthType::ApiKey,
            authenticated: false,
            auth_detail: None,
            default_model: Some("gpt-5.5".to_string()),
            recommended: false,
        };

        assert_eq!(provider.label(), "OpenCode - paste API key");
    }

    #[test]
    fn provider_api_key_prompt_hides_pasted_key() {
        let theme = Theme::for_dark_background(true);
        let empty = provider_api_key_input_line("", theme);
        assert_eq!(empty.spans[1].content, "paste API key");

        let pasted = provider_api_key_input_line("sk-secret", theme);
        assert_eq!(pasted.spans[1].content, "[api key hidden]");
    }

    #[test]
    fn provider_api_key_url_uses_cursor_integrations_page() {
        assert_eq!(
            provider_api_key_url("cursor"),
            "https://cursor.com/dashboard/integrations"
        );
    }

    #[test]
    fn provider_menu_navigation_skips_section_headers() {
        let mut app = test_app();
        app.provider_menu_items = vec![
            ProviderMenuItem::Section("Codex".to_string()),
            ProviderMenuItem::Model(ProviderOption {
                provider_id: "codex".to_string(),
                model_id: "gpt-5.5".to_string(),
                label: "codex/gpt-5.5 (GPT-5.5)".to_string(),
                context_window: Some(1_000_000),
                default_reasoning: Some("medium".to_string()),
                reasoning_options: Vec::new(),
            }),
            ProviderMenuItem::Back,
        ];
        app.provider_state.select(Some(1));

        app.select_previous_provider_menu_item();
        assert_eq!(app.provider_state.selected(), Some(2));

        app.select_next_provider_menu_item();
        assert_eq!(app.provider_state.selected(), Some(1));
    }

    #[test]
    fn provider_search_line_shows_placeholder_and_query() {
        let theme = Theme::for_dark_background(true);
        let placeholder = provider_search_line("", theme);
        assert_eq!(placeholder.spans[1].content, "type to filter");

        let query = provider_search_line("codex", theme);
        assert_eq!(query.spans[1].content, "codex");
    }

    #[test]
    fn spinner_menu_items_include_all_spinners_and_back() {
        let mut items = WorkingSpinner::all()
            .iter()
            .copied()
            .map(ProviderMenuItem::Spinner)
            .chain(std::iter::once(ProviderMenuItem::Back))
            .collect::<Vec<_>>();

        assert_eq!(items.len(), WorkingSpinner::all().len() + 1);
        assert!(matches!(items.pop(), Some(ProviderMenuItem::Back)));
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
    fn active_turn_prompt_shortcuts_prioritize_queue_and_steer() {
        assert_eq!(
            active_turn_prompt_shortcut(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), true),
            Some(ActiveTurnPromptShortcut::Queue)
        );
        assert_eq!(
            active_turn_prompt_shortcut(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), true),
            Some(ActiveTurnPromptShortcut::Steer)
        );
    }

    #[test]
    fn active_turn_prompt_shortcuts_do_not_steal_empty_or_modified_keys() {
        assert_eq!(
            active_turn_prompt_shortcut(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), false),
            None
        );
        assert_eq!(
            active_turn_prompt_shortcut(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT), true),
            None
        );
        assert_eq!(
            active_turn_prompt_shortcut(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL), true),
            None
        );
    }

    #[test]
    fn composer_tab_queues_only_when_prompt_is_prepared() {
        assert!(composer_queue_key(
            KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
            true
        ));
        assert!(!composer_queue_key(
            KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
            false
        ));
        assert!(!composer_queue_key(
            KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT),
            true
        ));
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
    fn provider_auth_messages_reflect_status() {
        let signed_in = ProviderAuthResult {
            signed_in: true,
            account_id: Some("acct".to_string()),
        };
        assert_eq!(
            provider_auth_message(
                "Codex",
                "auth/codex/logout",
                "auth/codex/status",
                &signed_in
            ),
            "system: signed in with Codex account acct."
        );

        let signed_out = ProviderAuthResult {
            signed_in: false,
            account_id: None,
        };
        assert_eq!(
            provider_auth_message(
                "SuperGrok",
                "auth/supergrok/logout",
                "auth/supergrok/status",
                &signed_out
            ),
            "system: signed out of SuperGrok."
        );
        assert_eq!(
            provider_auth_message(
                "Codex",
                "auth/codex/logout",
                "auth/codex/logout",
                &signed_in
            ),
            "system: signed out of Codex."
        );
    }
}
