//! Theme file discovery.
//!
//! Scans configured directories for `*.css`, deduplicating by basename so a
//! project-local override wins over a user-global file. The active theme is
//! selected either by the `ROADER_THEME` environment variable (handy for
//! development) or by the persisted `tui.theme.active` config field.

use std::fs;
use std::path::{Path, PathBuf};

use super::overrides::ThemeOverrides;

#[derive(Debug, Clone)]
pub struct ThemeEntry {
    pub id: String,
    pub display_name: String,
    pub path: PathBuf,
}

/// Default directory list, expanded with `~`.
pub fn default_directories() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(home) = dirs::home_dir() {
        out.push(home.join(".roder").join("themes"));
    }
    out.push(PathBuf::from(".roder/themes"));
    // The repo's checked-in themes/ directory — useful for development and as
    // the "factory defaults" source.
    out.push(PathBuf::from("themes"));
    out
}

pub fn discover_themes(directories: &[PathBuf]) -> Vec<ThemeEntry> {
    let mut entries: Vec<ThemeEntry> = Vec::new();
    for dir in directories {
        let Ok(read) = fs::read_dir(dir) else {
            continue;
        };
        for entry in read.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("css") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let id = stem.to_string();
            if let Some(existing) = entries.iter_mut().find(|e| e.id == id) {
                // Later directory wins.
                existing.path = path.clone();
            } else {
                entries.push(ThemeEntry {
                    id: id.clone(),
                    display_name: id,
                    path,
                });
            }
        }
    }
    entries.sort_by(|a, b| a.id.cmp(&b.id));
    entries
}

/// Resolve the active theme. Order:
/// 1. `ROADER_THEME` env var (basename, no extension).
/// 2. The persisted `~/.roder/state.toml` `[tui].active_theme` value.
/// 3. The `config_active` argument (typically read from config).
/// 4. `"default"` if discoverable, else `None`.
pub fn active_theme<'a>(
    entries: &'a [ThemeEntry],
    config_active: Option<&str>,
) -> Option<&'a ThemeEntry> {
    let preferred = std::env::var("ROADER_THEME").ok();
    let persisted = super::state::read_active_theme();
    let want = preferred
        .as_deref()
        .or(persisted.as_deref())
        .or(config_active)
        .unwrap_or("default");
    entries
        .iter()
        .find(|e| e.id == want)
        .or_else(|| entries.iter().find(|e| e.id == "default"))
}

pub fn load_overrides(path: &Path) -> Option<ThemeOverrides> {
    let css = fs::read_to_string(path).ok()?;
    ThemeOverrides::from_css(&css).ok()
}

/// End-to-end: discover, pick active, parse. Returns `None` if anything fails;
/// callers should fall back to the compiled-in defaults.
pub fn load_active_theme(directories: &[PathBuf], config_active: Option<&str>) -> Option<ThemeOverrides> {
    let entries = discover_themes(directories);
    let entry = active_theme(&entries, config_active)?;
    load_overrides(&entry.path)
}

/// Look up a theme by id across the configured directories and parse it. This
/// is the entry point the command palette uses for a live switch — it bypasses
/// env-var/state-file resolution and loads exactly what the user picked.
pub fn load_theme_by_id(directories: &[PathBuf], id: &str) -> Option<ThemeOverrides> {
    let entries = discover_themes(directories);
    let entry = entries.iter().find(|e| e.id == id)?;
    load_overrides(&entry.path)
}
