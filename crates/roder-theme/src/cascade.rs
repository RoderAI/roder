//! Cascade and computed style.
//!
//! Given an ancestor chain ending at a target node, return the [`ComputedStyle`]
//! produced by walking every rule in the stylesheet, ranking matches by
//! `(important, specificity, source_order)`.

use ratatui::style::{Color, Modifier, Style};

use crate::ast::*;
use crate::node::{BoxModel, StyledNode};
use crate::properties::{
    BorderShape, Display, parse_border_radius, parse_border_shape, parse_border_shorthand,
    parse_color, parse_display, parse_padding,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontWeight {
    #[default]
    Normal,
    Bold,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontStyle {
    #[default]
    Normal,
    Italic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextDecoration {
    #[default]
    None,
    Underline,
    LineThrough,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ComputedStyle {
    pub color: Option<Color>,
    pub background: Option<Color>,
    pub font_weight: FontWeight,
    pub font_style: FontStyle,
    pub text_decoration: TextDecoration,
    pub display: Display,
    pub padding: [u16; 4],
    /// Border shape (`Plain`, `Rounded`, ...). `None` means the property was
    /// never declared so the renderer should keep its own default.
    pub border: Option<BorderShape>,
    pub border_color: Option<Color>,
}

impl ComputedStyle {
    pub fn to_ratatui(&self) -> Style {
        let mut style = Style::default();
        if let Some(fg) = self.color {
            style = style.fg(fg);
        }
        if let Some(bg) = self.background {
            style = style.bg(bg);
        }
        if self.font_weight == FontWeight::Bold {
            style = style.add_modifier(Modifier::BOLD);
        }
        if self.font_style == FontStyle::Italic {
            style = style.add_modifier(Modifier::ITALIC);
        }
        match self.text_decoration {
            TextDecoration::Underline => style = style.add_modifier(Modifier::UNDERLINED),
            TextDecoration::LineThrough => style = style.add_modifier(Modifier::CROSSED_OUT),
            TextDecoration::None => {}
        }
        style
    }

    pub fn box_model(&self) -> BoxModel {
        BoxModel {
            padding: self.padding,
        }
    }
}

/// Compute the style for the deepest node in `chain` against `sheet`.
pub fn compute<'a>(sheet: &Stylesheet, chain: &[&StyledNode<'a>]) -> ComputedStyle {
    debug_assert!(!chain.is_empty());
    // (important, specificity, source_order, declaration_index_within_rule, &Declaration)
    let mut hits: Vec<(bool, (u32, u32, u32), usize, usize, &Declaration)> = Vec::new();
    for rule in &sheet.rules {
        if !rule.selectors.iter().any(|sel| selector_matches(sel, chain)) {
            continue;
        }
        let spec = rule
            .selectors
            .iter()
            .filter(|sel| selector_matches(sel, chain))
            .map(|sel| sel.specificity())
            .max()
            .unwrap_or((0, 0, 0));
        for (idx, decl) in rule.declarations.iter().enumerate() {
            hits.push((decl.important, spec, rule.source_order, idx, decl));
        }
    }
    // Sort ascending; later writes win.
    hits.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(a.1.cmp(&b.1))
            .then(a.2.cmp(&b.2))
            .then(a.3.cmp(&b.3))
    });

    let mut computed = ComputedStyle::default();
    for (_, _, _, _, decl) in hits {
        apply_declaration(&mut computed, decl, &sheet.variables);
    }
    computed
}

fn apply_declaration(out: &mut ComputedStyle, decl: &Declaration, vars: &[(String, String)]) {
    let Value::Raw(raw) = &decl.value;
    let resolved = resolve_vars(raw, vars);
    match decl.name.as_str() {
        "color" => {
            if let Some(c) = parse_color(&resolved) {
                out.color = Some(c);
            }
        }
        "background-color" | "background" => {
            if let Some(c) = parse_color(&resolved) {
                out.background = Some(c);
            }
        }
        "font-weight" => {
            out.font_weight = match resolved.trim() {
                "bold" => FontWeight::Bold,
                _ => FontWeight::Normal,
            };
        }
        "font-style" => {
            out.font_style = match resolved.trim() {
                "italic" => FontStyle::Italic,
                _ => FontStyle::Normal,
            };
        }
        "text-decoration" => {
            out.text_decoration = match resolved.trim() {
                "underline" => TextDecoration::Underline,
                "line-through" => TextDecoration::LineThrough,
                _ => TextDecoration::None,
            };
        }
        "display" => {
            if let Some(d) = parse_display(&resolved) {
                out.display = d;
            }
        }
        "padding" => {
            if let Some(p) = parse_padding(&resolved) {
                out.padding = p;
            }
        }
        "border-style" => {
            if let Some(s) = parse_border_shape(&resolved) {
                out.border = Some(s);
            }
        }
        "border-radius" => {
            if let Some(s) = parse_border_radius(&resolved) {
                out.border = Some(s);
            }
        }
        "border-color" => {
            if let Some(c) = parse_color(&resolved) {
                out.border_color = Some(c);
            }
        }
        "border" => {
            let (shape, color) = parse_border_shorthand(&resolved);
            if let Some(s) = shape {
                out.border = Some(s);
            }
            if let Some(c) = color {
                out.border_color = Some(c);
            }
        }
        // Unknown property -> ignore. TODO: surface in inspector warnings.
        _ => {}
    }
}

/// `var(--name)` substitution. Single-pass with shallow cycle protection (depth
/// bound). Variable bodies are themselves resolved.
fn resolve_vars(input: &str, vars: &[(String, String)]) -> String {
    resolve_inner(input, vars, 0)
}

fn resolve_inner(input: &str, vars: &[(String, String)], depth: usize) -> String {
    if depth > 8 {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"var(") {
            if let Some(end) = input[i..].find(')') {
                let inner = &input[i + 4..i + end];
                let name = inner.trim().trim_start_matches("--").trim();
                let value = vars
                    .iter()
                    .rev()
                    .find(|(n, _)| n == name)
                    .map(|(_, v)| v.clone())
                    .unwrap_or_default();
                out.push_str(&resolve_inner(&value, vars, depth + 1));
                i += end + 1;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

// --- selector matching ---

fn selector_matches<'a>(sel: &Selector, chain: &[&StyledNode<'a>]) -> bool {
    // Walk parts right-to-left across the chain right-to-left.
    let mut chain_idx = chain.len();
    let parts = &sel.parts;
    // The rightmost part must match the target node.
    let mut part_idx = parts.len();
    while part_idx > 0 {
        let (simple, combinator_to_right) = &parts[part_idx - 1];
        // For the rightmost simple selector, only the target node may match.
        let must_match_immediate = part_idx == parts.len()
            || matches!(combinator_to_right, Combinator::Child);
        if chain_idx == 0 {
            return false;
        }
        if must_match_immediate {
            chain_idx -= 1;
            if !simple_matches(simple, chain[chain_idx]) {
                return false;
            }
        } else {
            // Descendant — search up the chain.
            let mut found = false;
            while chain_idx > 0 {
                chain_idx -= 1;
                if simple_matches(simple, chain[chain_idx]) {
                    found = true;
                    break;
                }
            }
            if !found {
                return false;
            }
        }
        part_idx -= 1;
    }
    true
}

fn simple_matches(simple: &SimpleSelector, node: &StyledNode<'_>) -> bool {
    if let Some(id) = &simple.id {
        if node.id != Some(id.as_str()) {
            return false;
        }
    }
    for class in &simple.classes {
        if !node.classes.iter().any(|c| c == class) {
            return false;
        }
    }
    for attr in &simple.attrs {
        match node.data.iter().find(|(k, _)| *k == attr.name) {
            Some((_, v)) => {
                if let Some(expected) = &attr.value {
                    if v != expected {
                        return false;
                    }
                }
            }
            None => return false,
        }
    }
    // Pseudos other than ones we explicitly support never match — they are
    // parsed for forward-compat but inert in this proof.
    for pseudo in &simple.pseudos {
        if pseudo != "root" {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn n<'a>(id: Option<&'a str>, classes: &[&'a str]) -> StyledNode<'a> {
        let mut node = StyledNode::container();
        if let Some(id) = id {
            node = node.id(id);
        }
        for c in classes {
            node = node.class(c);
        }
        node
    }

    #[test]
    fn id_beats_class_specificity() {
        let sheet = parse("#foo { color: red; } .bar { color: blue; }").unwrap();
        let node = n(Some("foo"), &["bar"]);
        let computed = compute(&sheet, &[&node]);
        // #foo (1,0,0) wins over .bar (0,1,0).
        assert_eq!(computed.color, Some(Color::Red));
    }

    #[test]
    fn later_rule_wins_at_same_specificity() {
        let sheet = parse(".a { color: red; } .a { color: blue; }").unwrap();
        let node = n(None, &["a"]);
        assert_eq!(compute(&sheet, &[&node]).color, Some(Color::Blue));
    }

    #[test]
    fn important_overrides_specificity() {
        let sheet = parse("#x { color: red; } .y { color: blue !important; }").unwrap();
        let mut node = n(Some("x"), &["y"]);
        node.classes.push("y");
        let computed = compute(&sheet, &[&node]);
        assert_eq!(computed.color, Some(Color::Blue));
    }

    #[test]
    fn descendant_combinator() {
        let sheet = parse("#a .b { color: red; }").unwrap();
        let parent = n(Some("a"), &[]);
        let mid = n(None, &[]);
        let target = n(None, &["b"]);
        let computed = compute(&sheet, &[&parent, &mid, &target]);
        assert_eq!(computed.color, Some(Color::Red));
    }

    #[test]
    fn child_combinator_requires_direct_parent() {
        let sheet = parse("#a > .b { color: red; }").unwrap();
        let parent = n(Some("a"), &[]);
        let mid = n(None, &[]);
        let target = n(None, &["b"]);
        // mid breaks the child relationship -> no match.
        assert_eq!(compute(&sheet, &[&parent, &mid, &target]).color, None);
        // direct child -> match.
        assert_eq!(compute(&sheet, &[&parent, &target]).color, Some(Color::Red));
    }

    #[test]
    fn attribute_matching() {
        let sheet = parse(r#".tool[data-status="error"] { color: red; }"#).unwrap();
        let mut ok = StyledNode::container();
        ok.classes.push("tool");
        ok.data.push(("data-status", "error"));
        assert_eq!(compute(&sheet, &[&ok]).color, Some(Color::Red));

        let mut ko = StyledNode::container();
        ko.classes.push("tool");
        ko.data.push(("data-status", "ok"));
        assert_eq!(compute(&sheet, &[&ko]).color, None);
    }

    #[test]
    fn variables_resolve() {
        let sheet = parse(":root { --accent: #ff0000; } .x { color: var(--accent); }").unwrap();
        let node = n(None, &["x"]);
        assert_eq!(
            compute(&sheet, &[&node]).color,
            Some(Color::Rgb(255, 0, 0))
        );
    }

    #[test]
    fn display_none_propagates() {
        let sheet = parse(".thinking { display: none; }").unwrap();
        let node = n(None, &["thinking"]);
        assert_eq!(compute(&sheet, &[&node]).display, Display::None);
    }
}
