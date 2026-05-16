//! Persisted user state for the theming system.
//!
//! Roder doesn't yet have a general-purpose user-state file, so the theme
//! picker writes a small TOML at `~/.roder/state.toml`:
//!
//! ```toml
//! [tui]
//! active_theme = "midnight"
//! ```
//!
//! Resolution order for the active theme is (highest first):
//!   1. `ROADER_THEME` env var
//!   2. `~/.roder/state.toml` `[tui].active_theme`
//!   3. config-provided value (none today)
//!   4. `"default"` if discoverable
//!
//! Failures (missing file, bad TOML, unwritable directory) are non-fatal —
//! the picker just falls back to the next layer.

use std::fs;
use std::path::PathBuf;

/// Path to the state file: `~/.roder/state.toml`. Returns `None` if the home
/// directory can't be determined (tests / locked-down environments).
pub fn state_file_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".roder").join("state.toml"))
}

/// Read `[tui].active_theme` from the state file. Any error returns `None`.
pub fn read_active_theme() -> Option<String> {
    let path = state_file_path()?;
    read_active_theme_from(&path)
}

/// Same as [`read_active_theme`] but reads from a specific path. Used in
/// tests to avoid touching the real `~/.roder` directory.
pub fn read_active_theme_from(path: &std::path::Path) -> Option<String> {
    let body = fs::read_to_string(path).ok()?;
    let parsed: toml::Value = toml::from_str(&body).ok()?;
    parsed
        .get("tui")?
        .get("active_theme")?
        .as_str()
        .map(|s| s.to_string())
}

/// Persist `[tui].active_theme` to `~/.roder/state.toml`. Creates the parent
/// directory if missing. Best-effort: returns the error rather than panicking
/// so the caller can surface it in the event log.
pub fn write_active_theme(theme_id: &str) -> std::io::Result<()> {
    let Some(path) = state_file_path() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no home directory available",
        ));
    };
    write_active_theme_to(&path, theme_id)
}

pub fn write_active_theme_to(path: &std::path::Path, theme_id: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    // Read-modify-write so we don't clobber other future [tui] keys.
    let mut doc: toml::Value = match fs::read_to_string(path) {
        Ok(body) => toml::from_str(&body).unwrap_or_else(|_| empty_doc()),
        Err(_) => empty_doc(),
    };
    let tui = doc
        .as_table_mut()
        .expect("doc is a table")
        .entry("tui".to_string())
        .or_insert_with(|| toml::Value::Table(toml::value::Table::new()));
    if let Some(table) = tui.as_table_mut() {
        table.insert(
            "active_theme".to_string(),
            toml::Value::String(theme_id.to_string()),
        );
    }
    let serialized = toml::to_string_pretty(&doc).unwrap_or_default();
    fs::write(path, serialized)
}

fn empty_doc() -> toml::Value {
    toml::Value::Table(toml::value::Table::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn round_trip_active_theme_through_a_temp_file() {
        let mut path = env::temp_dir();
        path.push(format!(
            "roder-state-{}-{}.toml",
            std::process::id(),
            uniq()
        ));
        let _ = fs::remove_file(&path);

        assert!(read_active_theme_from(&path).is_none());
        write_active_theme_to(&path, "midnight").unwrap();
        assert_eq!(read_active_theme_from(&path).as_deref(), Some("midnight"));

        // Overwrite preserves the key contract.
        write_active_theme_to(&path, "solarized").unwrap();
        assert_eq!(
            read_active_theme_from(&path).as_deref(),
            Some("solarized")
        );

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn write_preserves_unrelated_tui_keys() {
        let mut path = env::temp_dir();
        path.push(format!(
            "roder-state-preserve-{}-{}.toml",
            std::process::id(),
            uniq()
        ));
        let _ = fs::remove_file(&path);

        fs::write(
            &path,
            "[tui]\nspinner = \"dots\"\nactive_theme = \"old\"\n",
        )
        .unwrap();
        write_active_theme_to(&path, "new").unwrap();
        let body = fs::read_to_string(&path).unwrap();
        assert!(body.contains("spinner"));
        assert!(body.contains("\"new\""));

        let _ = fs::remove_file(&path);
    }

    fn uniq() -> u128 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    }
}
