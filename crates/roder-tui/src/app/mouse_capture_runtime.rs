use std::io::Write;

use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
};

use crate::mouse::MouseCaptureEvent;

pub(super) fn apply_mouse_capture_event<W: Write>(
    writer: &mut W,
    event: MouseCaptureEvent,
) -> anyhow::Result<()> {
    match event {
        MouseCaptureEvent::CaptureEnabled => execute!(writer, EnableMouseCapture)?,
        MouseCaptureEvent::CaptureDisabled => execute!(writer, DisableMouseCapture)?,
    }
    Ok(())
}
