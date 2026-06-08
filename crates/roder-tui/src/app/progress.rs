//! Drives the terminal's native progress indicator (the OSC 9;4 "ConEmu"
//! progress protocol) from turn lifecycle state.
//!
//! We never report a numeric percentage — inference has no meaningful
//! `[===>]`-style completion ratio — so we only ever use the discrete states:
//! indeterminate (animated loader) while a turn runs, paused while we wait on
//! the user, error after a failed turn, and cleared when idle. On Ghostty this
//! renders as the progress line under the tab bar (and the Dock badge on
//! macOS); on Windows Terminal / ConEmu it drives the taskbar. Terminals that
//! don't understand the sequence silently ignore it, so it is always safe to
//! emit.

use std::io::{self, Write};

/// The progress state we want the terminal to display. Maps onto the OSC 9;4
/// `state` parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum TerminalProgress {
    /// No active work — clears any existing indicator (state 0).
    #[default]
    Idle,
    /// A turn is running with no measurable percentage — animated /
    /// indeterminate loader (state 3).
    Working,
    /// Work is paused awaiting the user, e.g. a tool approval — warning /
    /// paused indicator (state 4).
    Paused,
    /// The last turn failed — error indicator (state 2). Persists until the
    /// next turn starts so a failure stays visible after the fact.
    Error,
}

impl TerminalProgress {
    /// The OSC 9;4 `state` code for this progress state.
    fn osc_state(self) -> u8 {
        match self {
            TerminalProgress::Idle => 0,
            TerminalProgress::Error => 2,
            TerminalProgress::Working => 3,
            TerminalProgress::Paused => 4,
        }
    }

    /// The full OSC 9;4 control sequence that asks the terminal to show this
    /// state. The progress field is always `0`: states 3 and 4 ignore it and
    /// states 0 and 2 don't use it.
    fn osc_sequence(self) -> String {
        format!("\x1b]9;4;{};0\x1b\\", self.osc_state())
    }
}

/// Tracks the desired terminal-progress state and emits an OSC 9;4 sequence
/// only when it changes, so we never re-write the same state on every frame.
#[derive(Debug, Default)]
pub(super) struct ProgressReporter {
    desired: TerminalProgress,
    emitted: Option<TerminalProgress>,
}

impl ProgressReporter {
    /// Record the state we'd like the terminal to display. Cheap; the actual
    /// write happens in [`ProgressReporter::flush`].
    pub(super) fn set(&mut self, state: TerminalProgress) {
        self.desired = state;
    }

    /// Write the OSC sequence to `writer` if the desired state has changed
    /// since the last flush. Returns whether anything was written.
    pub(super) fn flush<W: Write>(&mut self, writer: &mut W) -> io::Result<bool> {
        if self.emitted == Some(self.desired) {
            return Ok(false);
        }
        write!(writer, "{}", self.desired.osc_sequence())?;
        writer.flush()?;
        self.emitted = Some(self.desired);
        Ok(true)
    }

    /// Force the indicator to clear and flush it immediately, regardless of the
    /// current state. Used on shutdown so we never leave a stale loader behind.
    pub(super) fn clear<W: Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.set(TerminalProgress::Idle);
        self.flush(writer)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flush_to_string(reporter: &mut ProgressReporter) -> String {
        let mut buf = Vec::new();
        reporter.flush(&mut buf).expect("write progress");
        String::from_utf8(buf).expect("utf8")
    }

    #[test]
    fn each_state_maps_to_its_osc_code() {
        assert_eq!(TerminalProgress::Idle.osc_sequence(), "\x1b]9;4;0;0\x1b\\");
        assert_eq!(TerminalProgress::Error.osc_sequence(), "\x1b]9;4;2;0\x1b\\");
        assert_eq!(
            TerminalProgress::Working.osc_sequence(),
            "\x1b]9;4;3;0\x1b\\"
        );
        assert_eq!(
            TerminalProgress::Paused.osc_sequence(),
            "\x1b]9;4;4;0\x1b\\"
        );
    }

    #[test]
    fn flush_emits_only_on_change() {
        let mut reporter = ProgressReporter::default();

        reporter.set(TerminalProgress::Working);
        assert_eq!(flush_to_string(&mut reporter), "\x1b]9;4;3;0\x1b\\");

        // Same state again writes nothing.
        reporter.set(TerminalProgress::Working);
        assert_eq!(flush_to_string(&mut reporter), "");

        reporter.set(TerminalProgress::Error);
        assert_eq!(flush_to_string(&mut reporter), "\x1b]9;4;2;0\x1b\\");
    }

    #[test]
    fn clear_resets_to_idle() {
        let mut reporter = ProgressReporter::default();
        reporter.set(TerminalProgress::Working);
        let _ = flush_to_string(&mut reporter);

        let mut buf = Vec::new();
        reporter.clear(&mut buf).expect("clear");
        assert_eq!(String::from_utf8(buf).unwrap(), "\x1b]9;4;0;0\x1b\\");
    }

    #[test]
    fn idle_from_start_still_emits_once() {
        // The first flush always emits so the terminal starts from a known
        // state even if we were never working.
        let mut reporter = ProgressReporter::default();
        assert_eq!(flush_to_string(&mut reporter), "\x1b]9;4;0;0\x1b\\");
        assert_eq!(flush_to_string(&mut reporter), "");
    }
}
