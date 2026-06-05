use std::{
    io,
    time::{Duration, Instant},
};

#[cfg(not(windows))]
use crossterm::event::{PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags};
use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyCode, KeyEvent, KeyModifiers, KeyboardEnhancementFlags, MouseButton, MouseEvent,
        MouseEventKind,
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
    restored: bool,
}

impl TerminalSession {
    pub fn enter() -> anyhow::Result<Self> {
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
        self.restored = true;
        Ok(())
    }
}

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
            keyboard_enhancement_flags()
                .contains(KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS)
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
