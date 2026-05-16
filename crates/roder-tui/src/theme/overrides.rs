//! Theme overrides derived from a parsed stylesheet.
//!
//! The compiled-in `Theme` palette pulls each of its color fields from a
//! conventional `:root` variable name. A theme that declares the variable
//! retints the corresponding surface; a theme that doesn't declares stays at
//! the baseline. This is the proof's pragmatic stand-in for full per-node
//! cascade — it lets every theme file in `themes/` retint the running TUI
//! without a sprawling renderer refactor.
//!
//! See `themes/default.css` for the full list of conventional variables.

use std::collections::BTreeSet;

use ratatui::style::Color;
use roder_theme::{BorderShape, StyleMap, Stylesheet};

use crate::theme::node_tree::CrossCuttingTag;

/// All conventional variable names this proof understands. Themes may set any
/// subset; unset variables fall back to the compiled-in palette.
pub const KNOWN_VARS: &[&str] = &[
    "background",
    "text",
    "muted",
    "subtle",
    "accent",
    "accent-soft",
    "tool",
    "diff-added",
    "diff-added-bg",
    "diff-removed",
    "diff-removed-bg",
    "shell",
    "error",
    "border",
    "mode-plan",
    "mode-default",
    "mode-accept-edits",
    "mode-bypass",
    "selection-bg",
    "selection-fg",
    "dialog",
    "dialog-bg",
    "dialog-shadow",
    "dialog-key-bg",
];

#[derive(Debug, Clone, Default)]
pub struct ThemeOverrides {
    pub colors: Vec<(String, Color)>,
    /// Classes that the theme requests be hidden via `display: none`.
    pub hidden_classes: BTreeSet<String>,
    pub style_map: StyleMap,
    /// Color to paint the whole terminal background with. `None` means the
    /// theme is transparent — the terminal's own background bleeds through.
    /// Themes set this via `:root { background: ... }` (or
    /// `background-color`), `:root { --background: ... }`, or
    /// `#body { background: ... }`. The literal value `transparent` (or
    /// `reset`) is treated the same as omitting it entirely.
    pub background: Option<Color>,
    /// Border shape for every framed widget (composer, popup, dialog,
    /// tool detail, palette, diff). Set via `:root { border-radius: ... }`,
    /// `:root { border-style: ... }`, `:root { border: ... }`, or
    /// `#body { border-style: ... }`. `None` keeps the compiled-in default
    /// (`Rounded`).
    pub border_shape: Option<BorderShape>,
}

impl ThemeOverrides {
    pub fn from_css(input: &str) -> Result<Self, roder_theme::ParseError> {
        let sheet = roder_theme::parse(input)?;
        Ok(Self::from_sheet(sheet))
    }

    pub fn from_sheet(sheet: Stylesheet) -> Self {
        let mut colors = Vec::new();
        for (name, raw) in &sheet.variables {
            if !KNOWN_VARS.iter().any(|k| *k == name.as_str()) {
                // Unknown variable — ignore for the proof; the inspector would
                // warn here in the full RFC.
                continue;
            }
            // The variable body might itself contain var(...) references —
            // resolve via the style map's cascade. Cheap because it just
            // performs string substitution against the `:root` table.
            let resolved = resolve_var_chain(raw, &sheet.variables, 0);
            if let Some(c) = roder_theme::properties::parse_color(&resolved) {
                colors.push((name.clone(), c));
            }
        }
        let hidden = hidden_classes_from(&sheet);
        let style_map = StyleMap::new(sheet.clone());
        let background = resolve_body_background(&style_map, &colors);
        let border_shape = resolve_border_shape(&style_map, &sheet);
        ThemeOverrides {
            colors,
            hidden_classes: hidden,
            style_map,
            background,
            border_shape,
        }
    }

    pub fn color(&self, name: &str) -> Option<Color> {
        self.colors
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, c)| *c)
    }

    pub fn hides(&self, class: &str) -> bool {
        self.hidden_classes.contains(class)
    }

    pub fn hides_cross_cutting(&self, tag: CrossCuttingTag) -> bool {
        self.hides(tag.class_name())
    }
}

/// Compute the body background from either `#body { background: ... }` (via
/// the style map) or the `background` variable on `:root`. `Color::Reset`
/// (which `parse_color` returns for `transparent` / `reset`) collapses back
/// to `None` so renderers know to skip painting.
fn resolve_body_background(style_map: &StyleMap, colors: &[(String, Color)]) -> Option<Color> {
    let computed = style_map.computed(&roder_theme::StyledNode::container().id("body"));
    if let Some(c) = computed.background {
        return (c != Color::Reset).then_some(c);
    }
    let var = colors
        .iter()
        .rev()
        .find(|(n, _)| n == "background")
        .map(|(_, c)| *c)?;
    (var != Color::Reset).then_some(var)
}

/// Compute the global border shape. Precedence (highest first):
/// 1. `#body` computed style — the "DOM-ish" form.
/// 2. `:root { border-style: ... }` / `border-radius` / `border` shorthand,
///    captured by the parser as synthetic variables of the same name.
fn resolve_border_shape(style_map: &StyleMap, sheet: &Stylesheet) -> Option<BorderShape> {
    let computed = style_map.computed(&roder_theme::StyledNode::container().id("body"));
    if let Some(s) = computed.border {
        return Some(s);
    }
    // Fall back to scanning the synthetic variables the parser stashed for
    // `:root { border-* }` declarations.
    let lookup = |name: &str| -> Option<String> {
        sheet
            .variables
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.clone())
    };
    if let Some(raw) = lookup("border-radius")
        && let Some(s) = roder_theme::properties::parse_border_radius(&raw)
    {
        return Some(s);
    }
    if let Some(raw) = lookup("border-style")
        && let Some(s) = roder_theme::properties::parse_border_shape(&raw)
    {
        return Some(s);
    }
    if let Some(raw) = lookup("border") {
        let (shape, _color) = roder_theme::properties::parse_border_shorthand(&raw);
        return shape;
    }
    None
}

fn resolve_var_chain(raw: &str, vars: &[(String, String)], depth: usize) -> String {
    if depth > 8 {
        return raw.to_string();
    }
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("var(") {
        if let Some(end) = rest.find(')') {
            let name = rest[..end].trim().trim_start_matches("--");
            if let Some((_, v)) = vars.iter().rev().find(|(n, _)| n == name) {
                return resolve_var_chain(v, vars, depth + 1);
            }
        }
    }
    trimmed.to_string()
}

/// Classes that the stylesheet hides outright via `display: none;`.
/// Computed by asking the style map about a fake node carrying each class.
pub fn hidden_classes_from(sheet: &Stylesheet) -> BTreeSet<String> {
    let map = StyleMap::new(sheet.clone());
    let mut out = BTreeSet::new();
    for class in HIDABLE_CLASSES {
        let node = roder_theme::StyledNode::container().class(class);
        let computed = map.computed(&node);
        if matches!(computed.display, roder_theme::Display::None) {
            out.insert((*class).to_string());
        }
    }
    out
}

/// Convenience for external callers (tests).
pub fn hidden_classes(input: &str) -> BTreeSet<String> {
    match roder_theme::parse(input) {
        Ok(sheet) => hidden_classes_from(&sheet),
        Err(_) => BTreeSet::new(),
    }
}

/// Closed list of classes whose `display: none` we honor in this proof. The
/// renderer code checks against this exact list — adding a new entry means
/// teaching one render path to respect it.
pub const HIDABLE_CLASSES: &[&str] = &[
    "timeline-thinking",
    "timeline-system",
    "timeline-error",
    "timeline-tool",
    "timeline-assistant",
    "timeline-user",
];
