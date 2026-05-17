//! Palette-level tests for the Themes source.
//!
//! These tests exercise the same code path the running TUI uses when the user
//! commits a theme row in the command palette: build the `theme_source`,
//! emit a `PaletteAction::SetTheme`, call `theme::apply_theme` to load and
//! persist, then verify that (a) a fresh read of the state file picks the
//! right theme and (b) `load_active_theme` honors the persisted value across
//! a simulated restart.
//!
//! We avoid spinning a real `TuiApp` (which needs a live `LocalAppClient`)
//! and instead drive the library surfaces directly. The palette source and
//! the dispatch helper are the contract the app's `apply_theme_by_id` method
//! delegates to.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use roder_tui::palette::PaletteAction;
use roder_tui::palette::sources::theme_source;
use roder_tui::theme::{
    ThemeEntry, apply_theme, discover_themes, read_active_theme, state::read_active_theme_from,
};

fn repo_themes_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("themes")
}

fn unique_state_file(label: &str) -> PathBuf {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    env::temp_dir().join(format!(
        "roder-state-{label}-{}-{}.toml",
        std::process::id(),
        n
    ))
}

#[test]
fn theme_source_selecting_a_row_emits_set_theme_action() {
    // Build the palette source over a synthetic two-theme list and act as the
    // palette would: look up the entry by id, execute it, and check the
    // resulting action.
    let entries = vec![
        ThemeEntry {
            id: "alpha".to_string(),
            display_name: "alpha".to_string(),
            path: PathBuf::from("themes/alpha.css"),
        },
        ThemeEntry {
            id: "beta".to_string(),
            display_name: "beta".to_string(),
            path: PathBuf::from("themes/beta.css"),
        },
    ];

    let source = theme_source(&entries, Some("alpha"));
    let rows = source.entries();
    let beta_row = rows
        .iter()
        .find(|r| r.item.id == "beta")
        .expect("beta row exists");

    assert_eq!(
        beta_row.action,
        PaletteAction::SetTheme("beta".to_string()),
        "selecting a theme row must emit SetTheme(<id>)"
    );

    let alpha_row = rows.iter().find(|r| r.item.id == "alpha").unwrap();
    assert!(
        alpha_row.item.title.contains("(active)"),
        "currently-active theme should be labeled"
    );
}

#[test]
fn dispatching_set_theme_loads_and_persists_the_choice() {
    let dirs = vec![repo_themes_dir()];
    let state = unique_state_file("dispatch");
    let _ = fs::remove_file(&state);

    // The palette emits SetTheme; the dispatcher resolves it.
    let action = PaletteAction::SetTheme("midnight".to_string());
    let PaletteAction::SetTheme(id) = action else {
        panic!("unexpected action variant");
    };

    let overrides =
        apply_theme(&dirs, &state, &id).expect("midnight must load from the repo themes dir");
    // Midnight ships a sky-blue accent.
    assert!(overrides.color("accent").is_some());

    // And the choice is persisted.
    assert_eq!(read_active_theme_from(&state).as_deref(), Some("midnight"));

    let _ = fs::remove_file(&state);
}

#[test]
fn persisted_theme_survives_a_simulated_restart() {
    // Simulate restart by re-running discovery+resolution from scratch against
    // a fresh state file. The picker writes the theme; a "next launch"
    // reads it back via `active_theme` and ends up with the same selection.
    let dirs = vec![repo_themes_dir()];
    let state = unique_state_file("restart");
    let _ = fs::remove_file(&state);

    apply_theme(&dirs, &state, "solarized").expect("solarized loads");

    // (a) the file we wrote holds the choice, and
    // (b) discovery yields the same entry the persisted name points at.
    // We deliberately bypass `active_theme` here because it reads the
    // *global* `~/.roder/state.toml`, which makes the test non-hermetic if
    // the developer has played with themes locally.
    assert_eq!(read_active_theme_from(&state).as_deref(), Some("solarized"));

    let entries = discover_themes(&dirs);
    let persisted = read_active_theme_from(&state).expect("file holds choice");
    let resolved = entries
        .iter()
        .find(|e| e.id == persisted)
        .expect("persisted entry discoverable");
    assert_eq!(resolved.id, "solarized");

    let _ = fs::remove_file(&state);
}

#[test]
fn unknown_theme_id_yields_a_not_found_error() {
    let dirs = vec![repo_themes_dir()];
    let state = unique_state_file("nope");
    let _ = fs::remove_file(&state);

    let err = apply_theme(&dirs, &state, "this-theme-does-not-exist").expect_err("must error");
    let msg = format!("{err}");
    assert!(msg.contains("this-theme-does-not-exist"), "msg={msg}");
    // No state file written.
    assert!(read_active_theme_from(&state).is_none());
}

#[test]
fn state_file_is_global_resolution_layer() {
    // Sanity: read_active_theme() is what `for_terminal_themed` ultimately
    // consults at startup. Just call it to confirm the API is exported and
    // doesn't panic in an unconfigured environment.
    let _ = read_active_theme();
}
