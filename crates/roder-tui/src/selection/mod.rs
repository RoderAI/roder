pub mod clipboard;
pub mod keymap;
pub mod offset;
pub mod range;

pub use clipboard::{ClipboardSink, copy_selection};
pub use keymap::{SelectionCommand, selection_command_for_key};
pub use offset::{OffsetRange, selected_offset_text};
pub use range::{Point, Range, selected_text};
