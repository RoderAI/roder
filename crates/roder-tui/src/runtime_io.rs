use std::{
    io,
    time::{Duration, Instant},
};

#[cfg(any(not(windows), test))]
use crossterm::event::KeyboardEnhancementFlags;
#[cfg(not(windows))]
use crossterm::event::{PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags};
use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use roder_api_transcript::{RecordedMouseButton, RecordedMouseEventKind, RecordedUiInput};

pub type LiveTerminal = Terminal<CrosstermBackend<io::Stdout>>;

pub trait TuiClock {
    fn now(&self) -> Instant;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl TuiClock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

pub trait TuiInputSource {
    fn poll(&self, timeout: Duration) -> io::Result<bool>;
    fn read(&mut self) -> io::Result<Event>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CrosstermInputSource;

impl TuiInputSource for CrosstermInputSource {
    fn poll(&self, timeout: Duration) -> io::Result<bool> {
        event::poll(timeout)
    }

    fn read(&mut self) -> io::Result<Event> {
        event::read()
    }
}

pub trait TuiInputRecorder {
    fn record_input(&mut self, input: RecordedUiInput) -> anyhow::Result<()>;
}

impl<F> TuiInputRecorder for F
where
    F: FnMut(RecordedUiInput) -> anyhow::Result<()>,
{
    fn record_input(&mut self, input: RecordedUiInput) -> anyhow::Result<()> {
        self(input)
    }
}

pub struct RecordingInputSource<S, R> {
    inner: S,
    recorder: R,
}

impl<S, R> RecordingInputSource<S, R> {
    pub fn new(inner: S, recorder: R) -> Self {
        Self { inner, recorder }
    }

    pub fn into_inner(self) -> S {
        self.inner
    }
}

impl<S, R> TuiInputSource for RecordingInputSource<S, R>
where
    S: TuiInputSource,
    R: TuiInputRecorder,
{
    fn poll(&self, timeout: Duration) -> io::Result<bool> {
        self.inner.poll(timeout)
    }

    fn read(&mut self) -> io::Result<Event> {
        let event = self.inner.read()?;
        if let Some(input) = recorded_ui_input_from_event(&event) {
            self.recorder
                .record_input(input)
                .map_err(io::Error::other)?;
        }
        Ok(event)
    }
}

pub fn recorded_ui_input_from_event(event: &Event) -> Option<RecordedUiInput> {
    match event {
        Event::Key(key) => Some(recorded_key_input(*key)),
        Event::Paste(text) => Some(RecordedUiInput::Paste { text: text.clone() }),
        Event::Mouse(mouse) => recorded_mouse_input(*mouse),
        Event::Resize(cols, rows) => Some(RecordedUiInput::Resize {
            cols: *cols,
            rows: *rows,
        }),
        _ => None,
    }
}

fn recorded_key_input(key: KeyEvent) -> RecordedUiInput {
    let (code, char) = recorded_key_code(key.code);
    RecordedUiInput::Key {
        code,
        char,
        modifiers: recorded_modifiers(key.modifiers),
    }
}

fn recorded_key_code(code: KeyCode) -> (String, Option<char>) {
    match code {
        KeyCode::Backspace => ("backspace".to_string(), None),
        KeyCode::Enter => ("enter".to_string(), None),
        KeyCode::Left => ("left".to_string(), None),
        KeyCode::Right => ("right".to_string(), None),
        KeyCode::Up => ("up".to_string(), None),
        KeyCode::Down => ("down".to_string(), None),
        KeyCode::Home => ("home".to_string(), None),
        KeyCode::End => ("end".to_string(), None),
        KeyCode::PageUp => ("page-up".to_string(), None),
        KeyCode::PageDown => ("page-down".to_string(), None),
        KeyCode::Tab => ("tab".to_string(), None),
        KeyCode::BackTab => ("back-tab".to_string(), None),
        KeyCode::Delete => ("delete".to_string(), None),
        KeyCode::Insert => ("insert".to_string(), None),
        KeyCode::F(n) => (format!("f{n}"), None),
        KeyCode::Char(c) => ("char".to_string(), Some(c)),
        KeyCode::Null => ("null".to_string(), None),
        KeyCode::Esc => ("escape".to_string(), None),
        KeyCode::CapsLock => ("caps-lock".to_string(), None),
        KeyCode::ScrollLock => ("scroll-lock".to_string(), None),
        KeyCode::NumLock => ("num-lock".to_string(), None),
        KeyCode::PrintScreen => ("print-screen".to_string(), None),
        KeyCode::Pause => ("pause".to_string(), None),
        KeyCode::Menu => ("menu".to_string(), None),
        KeyCode::KeypadBegin => ("keypad-begin".to_string(), None),
        KeyCode::Media(media) => (format!("media:{media:?}"), None),
        KeyCode::Modifier(modifier) => (format!("modifier:{modifier:?}"), None),
    }
}

fn recorded_mouse_input(mouse: MouseEvent) -> Option<RecordedUiInput> {
    let kind = match mouse.kind {
        MouseEventKind::Down(button) => RecordedMouseEventKind::Down {
            button: recorded_mouse_button(button)?,
        },
        MouseEventKind::Up(button) => RecordedMouseEventKind::Up {
            button: recorded_mouse_button(button)?,
        },
        MouseEventKind::Drag(button) => RecordedMouseEventKind::Drag {
            button: recorded_mouse_button(button)?,
        },
        MouseEventKind::Moved => RecordedMouseEventKind::Moved,
        MouseEventKind::ScrollDown => RecordedMouseEventKind::ScrollDown,
        MouseEventKind::ScrollUp => RecordedMouseEventKind::ScrollUp,
        MouseEventKind::ScrollLeft => RecordedMouseEventKind::ScrollLeft,
        MouseEventKind::ScrollRight => RecordedMouseEventKind::ScrollRight,
    };

    Some(RecordedUiInput::Mouse {
        kind,
        column: mouse.column,
        row: mouse.row,
        modifiers: recorded_modifiers(mouse.modifiers),
    })
}

fn recorded_mouse_button(button: MouseButton) -> Option<RecordedMouseButton> {
    match button {
        MouseButton::Left => Some(RecordedMouseButton::Left),
        MouseButton::Right => Some(RecordedMouseButton::Right),
        MouseButton::Middle => Some(RecordedMouseButton::Middle),
    }
}

fn recorded_modifiers(modifiers: KeyModifiers) -> Vec<String> {
    let mut out = Vec::new();
    if modifiers.contains(KeyModifiers::CONTROL) {
        out.push("control".to_string());
    }
    if modifiers.contains(KeyModifiers::ALT) {
        out.push("alt".to_string());
    }
    if modifiers.contains(KeyModifiers::SHIFT) {
        out.push("shift".to_string());
    }
    if modifiers.contains(KeyModifiers::SUPER) {
        out.push("super".to_string());
    }
    if modifiers.contains(KeyModifiers::HYPER) {
        out.push("hyper".to_string());
    }
    if modifiers.contains(KeyModifiers::META) {
        out.push("meta".to_string());
    }
    out
}

pub struct TerminalSession {
    terminal: LiveTerminal,
    keyboard_enhancements_active: bool,
    tmux_keys: TmuxExtendedKeysGuard,
    restored: bool,
}

impl TerminalSession {
    pub fn enter() -> anyhow::Result<Self> {
        // Prefer CSI-u modified-key reporting inside tmux so Shift+Enter is
        // distinguishable from Enter (crossterm understands CSI 13;2u, but drops
        // the default xterm form CSI 27;2;13~). Existing panes only pick this up
        // after a respawn, so we may re-exec once via tmux.
        tmux_ensure_extended_keys_for_shift_enter()?;
        let tmux_keys = TmuxExtendedKeysGuard::apply();
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(
            stdout,
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableMouseCapture,
        )?;
        let keyboard_enhancements_active = push_keyboard_enhancements(&mut stdout)?;
        let terminal = Terminal::new(CrosstermBackend::new(stdout))?;
        Ok(Self {
            terminal,
            keyboard_enhancements_active,
            tmux_keys,
            restored: false,
        })
    }

    pub fn terminal_mut(&mut self) -> &mut LiveTerminal {
        &mut self.terminal
    }

    pub fn restore(&mut self) -> anyhow::Result<()> {
        if self.restored {
            return Ok(());
        }
        disable_raw_mode()?;
        execute!(
            self.terminal.backend_mut(),
            DisableBracketedPaste,
            DisableMouseCapture,
        )?;
        pop_keyboard_enhancements(
            self.terminal.backend_mut(),
            self.keyboard_enhancements_active,
        )?;
        execute!(self.terminal.backend_mut(), LeaveAlternateScreen)?;
        self.terminal.show_cursor()?;
        self.tmux_keys.restore();
        self.restored = true;
        Ok(())
    }
}

/// Env marker set on the one-time tmux respawn so we do not loop.
const TMUX_KEYS_READY_ENV: &str = "RODER_TMUX_KEYS_READY";

/// Ensure this tmux pane can distinguish Shift+Enter from Enter.
///
/// tmux only applies `extended-keys` to panes created (or respawned) after the
/// option is set. When Roder starts in an existing pane with the option off,
/// we set the session options and respawn once into the same pane.
fn tmux_ensure_extended_keys_for_shift_enter() -> anyhow::Result<()> {
    if std::env::var_os("TMUX").is_none() {
        return Ok(());
    }
    if std::env::var_os(TMUX_KEYS_READY_ENV).is_some() {
        // Pane was recreated with the right key mode; TerminalSession::enter
        // still installs the restore guard.
        return Ok(());
    }

    let mode = tmux_display("#{pane_key_mode}").unwrap_or_default();
    let already_ext = mode.starts_with("Ext");
    let keys = tmux_show_option("extended-keys").unwrap_or_else(|| "off".to_string());
    let format = tmux_show_option("extended-keys-format").unwrap_or_else(|| "xterm".to_string());
    if already_ext && keys == "always" && format == "csi-u" {
        return Ok(());
    }

    // Capture previous values for restore after the respawned process exits.
    // Persist them in env so the child guard can restore on exit.
    if std::env::var_os("RODER_TMUX_PREV_EXTENDED_KEYS").is_none() {
        // SAFETY: single-threaded startup before any threads spawn.
        unsafe {
            std::env::set_var("RODER_TMUX_PREV_EXTENDED_KEYS", &keys);
            std::env::set_var("RODER_TMUX_PREV_EXTENDED_KEYS_FORMAT", &format);
        }
    }

    let _ = tmux_set_option("extended-keys", "always");
    let _ = tmux_set_option("extended-keys-format", "csi-u");

    if already_ext {
        return Ok(());
    }

    tmux_respawn_self_for_extended_keys()?;
    // respawn -k replaces this process; if we get here, fall through and run
    // without enhanced keys (Ctrl+J still works for newlines).
    Ok(())
}

fn tmux_respawn_self_for_extended_keys() -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut cmd = std::process::Command::new("tmux");
    cmd.arg("respawn-pane");
    cmd.arg("-k");
    cmd.arg("-e");
    cmd.arg(format!("{TMUX_KEYS_READY_ENV}=1"));
    if let Ok(prev) = std::env::var("RODER_TMUX_PREV_EXTENDED_KEYS") {
        cmd.arg("-e");
        cmd.arg(format!("RODER_TMUX_PREV_EXTENDED_KEYS={prev}"));
    }
    if let Ok(prev) = std::env::var("RODER_TMUX_PREV_EXTENDED_KEYS_FORMAT") {
        cmd.arg("-e");
        cmd.arg(format!("RODER_TMUX_PREV_EXTENDED_KEYS_FORMAT={prev}"));
    }
    // Preserve cwd.
    if let Ok(cwd) = std::env::current_dir() {
        cmd.arg("-c");
        cmd.arg(cwd);
    }
    cmd.arg("--");
    cmd.arg(exe);
    cmd.args(args);
    let status = cmd.status()?;
    if status.success() {
        // Parent is being replaced; exit so we do not double-run the TUI.
        std::process::exit(0);
    }
    Ok(())
}

/// While Roder is full-screen inside tmux, force modified-key reporting so
/// Shift+Enter reaches the app as a distinct key event.
///
/// Restores the previous session values on exit. No-op when not running under
/// tmux or when the `tmux` binary is unavailable.
struct TmuxExtendedKeysGuard {
    previous_keys: Option<String>,
    previous_format: Option<String>,
    active: bool,
}

impl TmuxExtendedKeysGuard {
    fn apply() -> Self {
        if std::env::var_os("TMUX").is_none() {
            return Self {
                previous_keys: None,
                previous_format: None,
                active: false,
            };
        }
        Self::apply_options_only()
    }

    fn apply_options_only() -> Self {
        let previous_keys = std::env::var("RODER_TMUX_PREV_EXTENDED_KEYS")
            .ok()
            .or_else(|| tmux_show_option("extended-keys"))
            .unwrap_or_else(|| "off".to_string());
        let previous_format = std::env::var("RODER_TMUX_PREV_EXTENDED_KEYS_FORMAT")
            .ok()
            .or_else(|| tmux_show_option("extended-keys-format"))
            .unwrap_or_else(|| "xterm".to_string());
        // `always` reports modified keys even without a keyboard-protocol
        // negotiation. `csi-u` matches Kitty-style CSI 13;2u that crossterm
        // already parses (the xterm form CSI 27;2;13~ is dropped by 0.29).
        let set_keys = tmux_set_option("extended-keys", "always");
        let set_format = tmux_set_option("extended-keys-format", "csi-u");
        if !set_keys && !set_format {
            return Self {
                previous_keys: None,
                previous_format: None,
                active: false,
            };
        }
        Self {
            previous_keys: Some(previous_keys),
            previous_format: Some(previous_format),
            active: true,
        }
    }

    fn restore(&mut self) {
        if !self.active {
            return;
        }
        if let Some(value) = self.previous_keys.take() {
            let _ = tmux_set_option("extended-keys", &value);
        }
        if let Some(value) = self.previous_format.take() {
            let _ = tmux_set_option("extended-keys-format", &value);
        }
        self.active = false;
    }
}

impl Drop for TmuxExtendedKeysGuard {
    fn drop(&mut self) {
        self.restore();
    }
}

fn tmux_show_option(name: &str) -> Option<String> {
    let output = std::process::Command::new("tmux")
        .args(["show-options", "-qv", name])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn tmux_set_option(name: &str, value: &str) -> bool {
    std::process::Command::new("tmux")
        .args(["set-option", name, value])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn tmux_display(format: &str) -> Option<String> {
    let output = std::process::Command::new("tmux")
        .args(["display-message", "-p", format])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

#[cfg(any(not(windows), test))]
pub(crate) fn keyboard_enhancement_flags() -> KeyboardEnhancementFlags {
    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
        | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
        // Without REPORT_ALTERNATE_KEYS, terminals that fully implement the Kitty
        // keyboard protocol (e.g. Ghostty) report shifted keys as the base key plus a
        // SHIFT modifier and omit the shifted codepoint. crossterm can only recover the
        // actual character (uppercase letters, shifted symbols like `$`) when the
        // alternate/shifted keycode is present, so request it here.
        | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
}

#[cfg(not(windows))]
fn push_keyboard_enhancements<W: io::Write>(writer: &mut W) -> io::Result<bool> {
    execute!(
        writer,
        PushKeyboardEnhancementFlags(keyboard_enhancement_flags())
    )?;
    Ok(true)
}

#[cfg(windows)]
fn push_keyboard_enhancements<W: io::Write>(_writer: &mut W) -> io::Result<bool> {
    Ok(false)
}

#[cfg(not(windows))]
fn pop_keyboard_enhancements<W: io::Write>(writer: &mut W, active: bool) -> io::Result<()> {
    if active {
        execute!(writer, PopKeyboardEnhancementFlags)?;
    }
    Ok(())
}

#[cfg(windows)]
fn pop_keyboard_enhancements<W: io::Write>(_writer: &mut W, _active: bool) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, KeyEventKind};
    use std::collections::VecDeque;

    #[test]
    fn system_clock_moves_forward() {
        let clock = SystemClock;
        assert!(clock.now() <= Instant::now());
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
        // Required so terminals that fully implement the Kitty protocol (Ghostty) send
        // the shifted codepoint, letting crossterm emit uppercase/shifted symbols.
        assert!(
            keyboard_enhancement_flags().contains(KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS)
        );
    }

    #[test]
    fn ui_input_records_keys_paste_mouse_and_resize() {
        assert_eq!(
            recorded_ui_input_from_event(&Event::Key(KeyEvent::new_with_kind(
                KeyCode::Char('p'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
                KeyEventKind::Press,
            ))),
            Some(RecordedUiInput::Key {
                code: "char".to_string(),
                char: Some('p'),
                modifiers: vec!["control".to_string(), "shift".to_string()],
            })
        );
        assert_eq!(
            recorded_ui_input_from_event(&Event::Paste("hello".to_string())),
            Some(RecordedUiInput::Paste {
                text: "hello".to_string()
            })
        );
        assert_eq!(
            recorded_ui_input_from_event(&Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 7,
                row: 3,
                modifiers: KeyModifiers::ALT,
            })),
            Some(RecordedUiInput::Mouse {
                kind: RecordedMouseEventKind::Down {
                    button: RecordedMouseButton::Left,
                },
                column: 7,
                row: 3,
                modifiers: vec!["alt".to_string()],
            })
        );
        assert_eq!(
            recorded_ui_input_from_event(&Event::Resize(120, 36)),
            Some(RecordedUiInput::Resize {
                cols: 120,
                rows: 36,
            })
        );
    }

    #[test]
    fn recording_input_source_records_before_returning_event() {
        struct FakeInputSource {
            events: VecDeque<Event>,
        }

        impl TuiInputSource for FakeInputSource {
            fn poll(&self, _timeout: Duration) -> io::Result<bool> {
                Ok(!self.events.is_empty())
            }

            fn read(&mut self) -> io::Result<Event> {
                self.events
                    .pop_front()
                    .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "no event"))
            }
        }

        let source = FakeInputSource {
            events: VecDeque::from([Event::Resize(80, 24)]),
        };
        let mut recorded = Vec::new();
        let mut source = RecordingInputSource::new(source, |input| {
            recorded.push(input);
            Ok(())
        });

        assert!(source.poll(Duration::ZERO).unwrap());
        assert_eq!(source.read().unwrap(), Event::Resize(80, 24));

        assert_eq!(
            recorded,
            vec![RecordedUiInput::Resize { cols: 80, rows: 24 }]
        );
    }
}
