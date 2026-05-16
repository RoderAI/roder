//! TUI side of the CSS theming system (proof-of-concept).
//!
//! This module wires `roder-theme` into the existing TUI. The full RFC asks
//! for a `StyledNode` tree threaded through every renderer; in this proof we
//! take a smaller bite:
//!
//! 1. We let themes expose a set of well-known `:root` variables and patch the
//!    existing `Theme` color fields from them. This is enough to retheme every
//!    surface that already pulls its colors from `Theme`.
//! 2. We let themes hide elements via `display: none` on the small set of
//!    classes the renderers consult.
//! 3. We expose a tiny [`StyleMap`] passthrough so per-surface tests can
//!    assert `.error { color: ... }` and friends.
//!
//! TODO(rfc): full `StyledNode` integration in every renderer.
//! TODO(rfc): hot-reload via `notify`.
//! TODO(rfc): inspector overlay (`Ctrl+Shift+I`).
//! TODO(rfc): extension `ProvidedService::ThemeContribution`.

pub mod discovery;
pub mod node_tree;
pub mod overrides;
pub mod state;

pub use discovery::{ThemeEntry, discover_themes, load_active_theme, load_theme_by_id};
pub use overrides::{ThemeOverrides, hidden_classes};
pub use roder_theme::{ComputedStyle, FontStyle, FontWeight, StyleMap, StyledNode, TextDecoration};
pub use state::{read_active_theme, write_active_theme};

use std::path::Path;

/// Apply a theme by id: load the stylesheet from `directories`, persist the
/// choice to `state_path` (so the next launch picks the same theme), and hand
/// the parsed [`ThemeOverrides`] back to the caller for live restyling.
///
/// This is the dispatch step that backs `PaletteAction::SetTheme`. It is split
/// out from the TUI's `apply_theme_by_id` method so library tests can drive
/// the same code path the palette uses without spinning a full `TuiApp`.
pub fn apply_theme(
    directories: &[std::path::PathBuf],
    state_path: &Path,
    id: &str,
) -> Result<ThemeOverrides, ApplyThemeError> {
    let overrides = load_theme_by_id(directories, id)
        .ok_or_else(|| ApplyThemeError::NotFound(id.to_string()))?;
    state::write_active_theme_to(state_path, id)
        .map_err(|err| ApplyThemeError::Persist(err.to_string()))?;
    Ok(overrides)
}

#[derive(Debug)]
pub enum ApplyThemeError {
    NotFound(String),
    Persist(String),
}

impl std::fmt::Display for ApplyThemeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(id) => write!(f, "theme {id} not found or failed to parse"),
            Self::Persist(err) => write!(f, "could not persist active theme: {err}"),
        }
    }
}

impl std::error::Error for ApplyThemeError {}
