pub mod builtins;
pub mod capture;
pub mod cursor;
pub mod drag;
pub mod focus;
pub mod handlers;
pub mod hover;
pub mod regions;
pub mod router;
pub mod scroll;

pub use builtins::{
    diff_hunk_region, palette_item_region, policy_approval_region, status_segment_region,
};
pub use capture::{CaptureController, CaptureEvent};
pub use cursor::{cursor_shape_escape, pointer_indicator};
pub use drag::{DragSelection, DragSelectionContent, DragSelectionController, drag_selection_text};
pub use focus::RegionFocusController;
pub use handlers::{RegionHandlerDispatcher, RoutedInteractiveEvent};
pub use hover::{HoverState, HoverStyleOverlay};
pub use regions::RegionFrame;
pub use router::MouseRouter;
pub use scroll::{ScrollCommand, ScrollController, ScrollTarget};
