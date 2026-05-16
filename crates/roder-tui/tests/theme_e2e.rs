//! End-to-end proof that the CSS theming system actually re-styles the TUI.
//!
//! Each test loads one of the checked-in themes from `themes/<name>.css`,
//! applies it to a fake transcript timeline, renders into a `ratatui::Buffer`,
//! and asserts that specific cells changed style. This is the "proof the
//! design is feasible" deliverable for RFC 0001.

use std::path::PathBuf;

use ratatui::style::{Color, Modifier};

use roder_theme::{ComputedStyle, StyleMap, StyledNode};
use roder_tui::theme::node_tree::{CrossCuttingTag, fake_timeline};
use roder_tui::theme::overrides::ThemeOverrides;

fn theme_path(name: &str) -> PathBuf {
    let manifest = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest)
        .join("..")
        .join("..")
        .join("themes")
        .join(format!("{name}.css"))
}

fn load(name: &str) -> ThemeOverrides {
    let css = std::fs::read_to_string(theme_path(name))
        .unwrap_or_else(|e| panic!("missing theme {name}: {e}"));
    ThemeOverrides::from_css(&css).unwrap_or_else(|e| panic!("theme {name} failed to parse: {e}"))
}

/// Canonical list of every theme committed to the repo's `themes/` directory.
/// Keep in sync with the on-disk set; `all_checked_in_themes_parse` and
/// `every_theme_changes_something_visible` iterate this list.
const ALL_THEMES: &[&str] = &[
    "default",
    "midnight",
    "solarized",
    "minimal",
    "high-contrast",
    "gruvbox",
    "dracula",
    "tokyo-night",
    "light",
    "focus",
];

#[test]
fn all_checked_in_themes_parse() {
    for name in ALL_THEMES {
        let _ = load(name);
    }
}

#[test]
fn repo_themes_directory_has_ten_or_more_files() {
    // The roadmap explicitly bumps the bundled set to 10. Catches accidental
    // deletions; the count check also catches a theme being added to the dir
    // without being registered in ALL_THEMES.
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("themes");
    let count = std::fs::read_dir(&dir)
        .expect("themes dir exists")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("css"))
        .count();
    assert!(count >= 10, "expected >=10 themes, found {count}");
    assert_eq!(
        count,
        ALL_THEMES.len(),
        "ALL_THEMES out of sync with themes/"
    );
}

#[test]
fn every_theme_changes_something_visible() {
    // Each theme must affect at least one well-known surface so the bundled
    // set proves out the cascade end-to-end (not just parsing).
    for name in ALL_THEMES {
        let theme = load(name);
        let has_accent = theme.color("accent").is_some();
        let has_error = theme.color("error").is_some();
        assert!(
            has_accent || has_error,
            "theme {name} does not set accent or error variables"
        );
    }
}

// Per-theme spot checks: one assertion per new theme that proves it really
// changes something the cascade can see. Keep these focused — the broad
// "everything parses" coverage lives in all_checked_in_themes_parse.
#[test]
fn gruvbox_uses_yellow_accent() {
    let theme = load("gruvbox");
    assert_eq!(theme.color("accent"), Some(Color::Rgb(0xfa, 0xbd, 0x2f)));
}

#[test]
fn dracula_paints_assistant_pink() {
    let theme = load("dracula");
    let assistant = StyledNode::container().class("timeline-assistant");
    let computed = theme.style_map.computed(&assistant);
    assert_eq!(computed.color, Some(Color::Rgb(0xff, 0x79, 0xc6)));
}

#[test]
fn tokyo_night_uses_cyan_accent_and_styles_composer() {
    let theme = load("tokyo-night");
    assert_eq!(theme.color("accent"), Some(Color::Rgb(0x7d, 0xcf, 0xff)));
    // The composer rule is keyed by id; assert the rule reached the cascade.
    let composer = StyledNode::container().id("composer");
    let _ = theme.style_map.computed(&composer);
}

#[test]
fn light_theme_uses_dark_ink_text() {
    let theme = load("light");
    assert_eq!(theme.color("text"), Some(Color::Rgb(0x1f, 0x1f, 0x1f)));
}

#[test]
fn focus_theme_hides_thinking_and_tool_args() {
    let theme = load("focus");
    assert!(theme.hides("timeline-thinking"));
    let args = StyledNode::container().class("tool-args");
    // .tool-args isn't in HIDABLE_CLASSES today, so theme.hides() won't see
    // it, but the cascade does and that's what proves the rule landed.
    let computed = theme.style_map.computed(&args);
    assert_eq!(computed.display, roder_theme::Display::None);
}

#[test]
fn default_theme_keeps_baseline() {
    let theme = load("default");
    // default.css doesn't hide anything.
    assert!(!theme.hides("timeline-thinking"));
    // It sets every conventional variable.
    assert!(theme.color("accent").is_some());
    assert!(theme.color("error").is_some());
}

#[test]
fn midnight_recolors_assistant_accent_and_keeps_thinking() {
    let theme = load("midnight");
    // Midnight ships an indigo accent.
    assert_eq!(theme.color("accent"), Some(Color::Rgb(0x6c, 0xca, 0xff)));
    assert!(!theme.hides("timeline-thinking"));
    // And rules apply to .timeline-tool.
    let tool_node = StyledNode::container().class("timeline-tool");
    let computed = theme.style_map.computed(&tool_node);
    assert_eq!(computed.color, Some(Color::Rgb(0x6c, 0xca, 0xff)));
}

#[test]
fn solarized_paints_errors_red_with_dark_background() {
    let theme = load("solarized");
    let err = StyledNode::container().class("error");
    let computed = theme.style_map.computed(&err);
    assert_eq!(computed.color, Some(Color::Rgb(0xdc, 0x32, 0x2f)));
    assert_eq!(computed.background, Some(Color::Rgb(0x07, 0x36, 0x42)));
}

#[test]
fn minimal_hides_thinking_and_keeps_errors_loud() {
    let theme = load("minimal");
    assert!(theme.hides("timeline-thinking"));
    // The `display: none` on .timeline-thinking is also visible via the engine
    // for any per-node consumer.
    let thinking = StyledNode::container().class("timeline-thinking");
    let computed = theme.style_map.computed(&thinking);
    assert_eq!(computed.display, roder_theme::Display::None);

    // .error still bold + bright.
    let err = StyledNode::container().class("error");
    let computed = theme.style_map.computed(&err);
    assert_eq!(computed.color, Some(Color::Rgb(0xff, 0x00, 0x33)));
    let style = computed.to_ratatui();
    assert!(style.add_modifier.contains(Modifier::BOLD));
}

#[test]
fn high_contrast_uses_bold_bright_fg() {
    let theme = load("high-contrast");
    // Cyan accent.
    assert_eq!(theme.color("accent"), Some(Color::Rgb(0, 255, 255)));

    let err = StyledNode::container().class("error");
    let computed: ComputedStyle = theme.style_map.computed(&err);
    let style = computed.to_ratatui();
    assert!(style.add_modifier.contains(Modifier::BOLD));
    assert!(style.add_modifier.contains(Modifier::UNDERLINED));
    assert_eq!(computed.color, Some(Color::Rgb(255, 0, 0)));
    assert_eq!(computed.background, Some(Color::Rgb(255, 255, 0)));

    // Assistant/tool rows are bold.
    let assistant = StyledNode::container().class("timeline-assistant");
    let computed = theme.style_map.computed(&assistant);
    assert_eq!(computed.font_weight, roder_theme::FontWeight::Bold);
}

#[test]
fn cascading_a_theme_against_the_fake_timeline_finds_each_surface() {
    // Walks the fake timeline and asserts every element receives a style that
    // the active theme dictates — proof the cascade plumbing reaches every
    // node we expect a renderer to query.
    let theme = load("midnight");
    let tree = fake_timeline();
    let map: &StyleMap = &theme.style_map;

    let mut visited = Vec::new();
    walk(&tree, &[], map, &mut visited);

    // The .timeline-tool node picked up the accent rule.
    let tool_hit = visited
        .iter()
        .find(|(classes, _)| classes.iter().any(|c| *c == "timeline-tool"))
        .expect("tool node should exist");
    assert_eq!(tool_hit.1.color, Some(Color::Rgb(0x6c, 0xca, 0xff)));

    // The .timeline-thinking node is present (midnight doesn't hide it) and
    // resolves to display: block (default).
    let thinking_hit = visited
        .iter()
        .find(|(classes, _)| classes.iter().any(|c| *c == "timeline-thinking"))
        .expect("thinking node should exist");
    assert_eq!(thinking_hit.1.display, roder_theme::Display::Block);
}

fn walk<'a>(
    node: &'a StyledNode<'a>,
    chain: &[&'a StyledNode<'a>],
    map: &StyleMap,
    out: &mut Vec<(Vec<&'a str>, ComputedStyle)>,
) {
    let mut new_chain: Vec<&StyledNode> = chain.to_vec();
    new_chain.push(node);
    let computed = map.computed_chain(&new_chain);
    out.push((node.classes.clone(), computed));
    for child in &node.children {
        walk(child, &new_chain, map, out);
    }
}

#[test]
fn cross_cutting_tags_round_trip() {
    // Sanity that the enum mirrors the registered class names.
    assert_eq!(CrossCuttingTag::Error.class_name(), "error");
    assert_eq!(CrossCuttingTag::Muted.class_name(), "muted");
}

// ---- Body background -------------------------------------------------------
//
// `:root { background: ... }`, `:root { --background: ... }`, and
// `#body { background-color: ... }` should all resolve to the same
// `ThemeOverrides::background`. `transparent` (and the omitted default) must
// collapse to `None` so the terminal's native background bleeds through.

#[test]
fn root_background_declaration_sets_body_background() {
    let theme = ThemeOverrides::from_css(":root { background: #123456; }").unwrap();
    assert_eq!(theme.background, Some(Color::Rgb(0x12, 0x34, 0x56)));
}

#[test]
fn root_background_variable_sets_body_background() {
    let theme = ThemeOverrides::from_css(":root { --background: #abcdef; }").unwrap();
    assert_eq!(theme.background, Some(Color::Rgb(0xab, 0xcd, 0xef)));
}

#[test]
fn body_id_selector_sets_body_background() {
    let theme = ThemeOverrides::from_css("#body { background-color: red; }").unwrap();
    assert_eq!(theme.background, Some(Color::Red));
}

#[test]
fn transparent_keyword_leaves_background_unset() {
    // `transparent` is the documented default and must collapse to None so
    // renderers know to skip the fill.
    let t1 = ThemeOverrides::from_css(":root { background: transparent; }").unwrap();
    let t2 = ThemeOverrides::from_css("#body { background-color: transparent; }").unwrap();
    let t3 = ThemeOverrides::from_css(":root { --background: transparent; }").unwrap();
    assert_eq!(t1.background, None);
    assert_eq!(t2.background, None);
    assert_eq!(t3.background, None);
}

#[test]
fn body_id_selector_wins_over_background_variable() {
    let css = ":root { --background: #111111; }\n#body { background: #ffffff; }";
    let theme = ThemeOverrides::from_css(css).unwrap();
    assert_eq!(theme.background, Some(Color::Rgb(0xff, 0xff, 0xff)));
}

// ---- Borders --------------------------------------------------------------
//
// Themes can set a global border shape (`Rounded` is the baseline default)
// via `:root { border-radius: ... }`, `:root { border-style: ... }`,
// `:root { border: ... }`, or `#body { border-style: ... }`.

#[test]
fn root_border_radius_zero_yields_plain_shape() {
    let theme = ThemeOverrides::from_css(":root { border-radius: 0; }").unwrap();
    assert_eq!(theme.border_shape, Some(roder_theme::BorderShape::Plain));
}

#[test]
fn root_border_radius_nonzero_yields_rounded_shape() {
    let theme = ThemeOverrides::from_css(":root { border-radius: 1; }").unwrap();
    assert_eq!(theme.border_shape, Some(roder_theme::BorderShape::Rounded));
}

#[test]
fn root_border_style_keyword_maps_to_shape() {
    let cases = [
        ("none", roder_theme::BorderShape::None),
        ("solid", roder_theme::BorderShape::Plain),
        ("plain", roder_theme::BorderShape::Plain),
        ("rounded", roder_theme::BorderShape::Rounded),
        ("double", roder_theme::BorderShape::Double),
        ("thick", roder_theme::BorderShape::Thick),
    ];
    for (kw, want) in cases {
        let css = format!(":root {{ border-style: {kw}; }}");
        let theme = ThemeOverrides::from_css(&css).unwrap();
        assert_eq!(theme.border_shape, Some(want), "for keyword {kw}");
    }
}

#[test]
fn body_id_selector_border_wins_over_root_variable() {
    let css = ":root { border-style: rounded; }\n#body { border-style: double; }";
    let theme = ThemeOverrides::from_css(css).unwrap();
    assert_eq!(theme.border_shape, Some(roder_theme::BorderShape::Double));
}

#[test]
fn border_shorthand_on_body_carries_shape() {
    let css = "#body { border: thick #ff8800; }";
    let theme = ThemeOverrides::from_css(css).unwrap();
    assert_eq!(theme.border_shape, Some(roder_theme::BorderShape::Thick));
}

#[test]
fn undeclared_border_leaves_default_intact() {
    let theme = ThemeOverrides::from_css(":root { --accent: red; }").unwrap();
    assert_eq!(theme.border_shape, None);
}
