//! Theme file discovery.
//!
//! Scans configured directories for `*.css`, deduplicating by basename so a
//! project-local override wins over a user-global file. The active theme is
//! selected either by the `ROADER_THEME` environment variable (handy for
//! development) or by the persisted `tui.theme.active` config field.

use std::fs;
use std::path::{Path, PathBuf};

fn roder_data_dir() -> Option<PathBuf> {
    std::env::var_os("RODER_DATA_DIR")
        .or_else(|| std::env::var_os("RODER_CONFIG_DIR"))
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".roder")))
}

use super::overrides::ThemeOverrides;

#[derive(Debug, Clone)]
pub struct ThemeEntry {
    pub id: String,
    pub display_name: String,
    pub path: PathBuf,
}

/// Default directory list, expanded with `~`.
///
/// Package-provided theme directories (roadmap phase 93) come first so that
/// user, project, and bundled themes shadow package themes on basename
/// collisions — `discover_themes` lets later directories win.
pub fn default_directories() -> Vec<PathBuf> {
    let mut out = package_theme_directories();
    if let Some(dir) = roder_data_dir() {
        out.push(dir.join("themes"));
    }
    out.push(PathBuf::from(".roder/themes"));
    // The repo's checked-in themes/ directory — useful for development and as
    // the "factory defaults" source.
    out.push(PathBuf::from("themes"));
    out
}

/// Theme directories contributed by installed packages. Empty when no
/// packages are installed; directories that vanish later are skipped by
/// `discover_themes` anyway.
fn package_theme_directories() -> Vec<PathBuf> {
    use roder_config::packages::{PackagePaths, package_theme_dirs};
    let workspace = std::env::current_dir().ok();
    package_theme_dirs(&PackagePaths::standard(workspace.as_deref()))
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
pub fn load_active_theme(
    directories: &[PathBuf],
    config_active: Option<&str>,
) -> Option<ThemeOverrides> {
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

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!(
            "roder-theme-discovery-{label}-{}-{nanos}",
            std::process::id()
        ))
    }

    #[test]
    fn user_theme_shadows_package_theme_with_same_basename() {
        // Mirrors the default_directories() ordering: package dirs first,
        // user dir later, so the user copy wins on basename collisions while
        // package-only themes still surface.
        let package_dir = unique_temp_dir("package");
        let user_dir = unique_temp_dir("user");
        fs::create_dir_all(&package_dir).unwrap();
        fs::create_dir_all(&user_dir).unwrap();
        fs::write(
            package_dir.join("aurora.css"),
            ":root { --accent: #ff0000; }",
        )
        .unwrap();
        fs::write(
            package_dir.join("pkg-only.css"),
            ":root { --accent: #00ff00; }",
        )
        .unwrap();
        fs::write(user_dir.join("aurora.css"), ":root { --accent: #0000ff; }").unwrap();

        let entries = discover_themes(&[package_dir.clone(), user_dir.clone()]);

        let aurora = entries
            .iter()
            .find(|entry| entry.id == "aurora")
            .expect("colliding theme discovered");
        assert_eq!(aurora.path, user_dir.join("aurora.css"));
        let pkg_only = entries
            .iter()
            .find(|entry| entry.id == "pkg-only")
            .expect("package-only theme discovered");
        assert_eq!(pkg_only.path, package_dir.join("pkg-only.css"));

        let _ = fs::remove_dir_all(&package_dir);
        let _ = fs::remove_dir_all(&user_dir);
    }

    #[test]
    fn default_directories_keep_user_project_and_bundled_dirs_last() {
        // Must not panic with no packages installed. Package dirs (possibly
        // none) are prepended, so the shadowing tail keeps its order:
        // data-dir themes, then .roder/themes, then the repo themes/.
        let dirs = default_directories();
        assert!(dirs.len() >= 2);
        assert_eq!(dirs.last(), Some(&PathBuf::from("themes")));
        let project = PathBuf::from(".roder/themes");
        let project_pos = dirs
            .iter()
            .position(|dir| dir == &project)
            .expect(".roder/themes present");
        assert_eq!(project_pos, dirs.len() - 2);
    }
}
