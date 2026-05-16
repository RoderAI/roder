//! Property value parsers.
//!
//! Only the subset listed in the proof: `color`, `background-color`,
//! `font-weight`, `font-style`, `text-decoration`, `display`, `padding`,
//! `border-style`, `border-radius`, `border` shorthand.

use ratatui::style::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Display {
    #[default]
    Block,
    Inline,
    None,
}

pub fn parse_display(input: &str) -> Option<Display> {
    match input.trim() {
        "block" => Some(Display::Block),
        "inline" => Some(Display::Inline),
        "none" => Some(Display::None),
        _ => None,
    }
}

/// Engine-side border vocabulary. The TUI layer maps each variant to
/// `ratatui::widgets::BorderType` (plus a flag for "no border at all").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BorderShape {
    /// No border drawn (also reclaims the 1-cell ring on each side).
    None,
    /// Single-line box drawing characters — the CSS `solid` keyword.
    #[default]
    Plain,
    /// Rounded corners. The default for Roder's existing widgets.
    Rounded,
    /// Double-line box drawing characters.
    Double,
    /// Heavy/thick single-line characters.
    Thick,
}

/// `border-style: none | plain | solid | single | rounded | double | thick`.
/// `solid` and `single` are aliases for `plain` so authors familiar with
/// either web CSS or the ratatui vocabulary land somewhere reasonable.
pub fn parse_border_shape(input: &str) -> Option<BorderShape> {
    match input.trim().to_ascii_lowercase().as_str() {
        "none" | "hidden" => Some(BorderShape::None),
        "plain" | "solid" | "single" => Some(BorderShape::Plain),
        "rounded" => Some(BorderShape::Rounded),
        "double" => Some(BorderShape::Double),
        "thick" | "heavy" | "bold" => Some(BorderShape::Thick),
        _ => None,
    }
}

/// `border-radius: <integer-cells>`. Zero collapses to a square (`Plain`);
/// any positive value rounds the corners. Terminals cannot draw a partial
/// arc so the cell radius is a boolean in practice — but we keep the integer
/// shape so authors can use a familiar spelling.
pub fn parse_border_radius(input: &str) -> Option<BorderShape> {
    let n: u16 = input.trim().parse().ok()?;
    Some(if n == 0 {
        BorderShape::Plain
    } else {
        BorderShape::Rounded
    })
}

/// `border: <one-or-two-tokens>` shorthand. Today we accept any subset of a
/// style keyword and a color, separated by whitespace. We deliberately do
/// not parse a width — every border in a terminal is one cell.
///
/// Returns `(shape, color)` with both optional. If neither is recognized,
/// the caller may treat the declaration as a no-op.
pub fn parse_border_shorthand(input: &str) -> (Option<BorderShape>, Option<Color>) {
    let mut shape = None;
    let mut color = None;
    for token in input.split_whitespace() {
        if shape.is_none()
            && let Some(s) = parse_border_shape(token)
        {
            shape = Some(s);
            continue;
        }
        if color.is_none()
            && let Some(c) = parse_color(token)
        {
            color = Some(c);
        }
    }
    (shape, color)
}

/// Parse a CSS color into a ratatui [`Color`].
///
/// Supported forms:
/// * `#rgb`, `#rrggbb`
/// * `rgb(r, g, b)` (0..=255 ints)
/// * `ansi(n)` (0..=255)
/// * a small named-color table
/// * `reset` -> [`Color::Reset`]
pub fn parse_color(input: &str) -> Option<Color> {
    let s = input.trim();
    if s.eq_ignore_ascii_case("reset") || s.eq_ignore_ascii_case("transparent") {
        // `transparent` resolves to the terminal default. Renderers that care
        // about "actually transparent vs. some color" should check for
        // [`Color::Reset`] explicitly.
        return Some(Color::Reset);
    }
    if let Some(rest) = s.strip_prefix('#') {
        return parse_hex_color(rest);
    }
    if let Some(rest) = s.strip_prefix("rgb(").and_then(|r| r.strip_suffix(')')) {
        let parts: Vec<_> = rest.split(',').map(|p| p.trim()).collect();
        if parts.len() == 3 {
            let r = parts[0].parse::<u8>().ok()?;
            let g = parts[1].parse::<u8>().ok()?;
            let b = parts[2].parse::<u8>().ok()?;
            return Some(Color::Rgb(r, g, b));
        }
        return None;
    }
    if let Some(rest) = s.strip_prefix("ansi(").and_then(|r| r.strip_suffix(')')) {
        let n = rest.trim().parse::<u16>().ok()?;
        if n <= 255 {
            return Some(Color::Indexed(n as u8));
        }
        return None;
    }
    named_color(s)
}

fn parse_hex_color(hex: &str) -> Option<Color> {
    match hex.len() {
        3 => {
            let mut bytes = [0u8; 3];
            for (i, c) in hex.chars().enumerate() {
                let v = c.to_digit(16)? as u8;
                bytes[i] = (v << 4) | v;
            }
            Some(Color::Rgb(bytes[0], bytes[1], bytes[2]))
        }
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some(Color::Rgb(r, g, b))
        }
        _ => None,
    }
}

fn named_color(name: &str) -> Option<Color> {
    let n = name.to_ascii_lowercase();
    Some(match n.as_str() {
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "white" => Color::White,
        "gray" | "grey" => Color::Gray,
        "darkgray" | "darkgrey" => Color::DarkGray,
        "lightred" => Color::LightRed,
        "lightgreen" => Color::LightGreen,
        "lightyellow" => Color::LightYellow,
        "lightblue" => Color::LightBlue,
        "lightmagenta" => Color::LightMagenta,
        "lightcyan" => Color::LightCyan,
        // A few common web names mapped to the closest ratatui colour. This is
        // intentionally minimal — themes that need precision should use #rrggbb.
        "orange" => Color::Rgb(255, 165, 0),
        "purple" => Color::Rgb(128, 0, 128),
        "pink" => Color::Rgb(255, 192, 203),
        "silver" => Color::Rgb(192, 192, 192),
        _ => return None,
    })
}

/// `padding` accepts 1..=4 cell counts.
pub fn parse_padding(input: &str) -> Option<[u16; 4]> {
    let parts: Vec<u16> = input
        .split_whitespace()
        .map(|p| p.parse::<u16>())
        .collect::<Result<_, _>>()
        .ok()?;
    Some(match parts.len() {
        1 => [parts[0]; 4],
        2 => [parts[0], parts[1], parts[0], parts[1]],
        3 => [parts[0], parts[1], parts[2], parts[1]],
        4 => [parts[0], parts[1], parts[2], parts[3]],
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_color_forms() {
        assert_eq!(parse_color("#fff"), Some(Color::Rgb(255, 255, 255)));
        assert_eq!(parse_color("#112233"), Some(Color::Rgb(0x11, 0x22, 0x33)));
        assert_eq!(parse_color("rgb(10, 20, 30)"), Some(Color::Rgb(10, 20, 30)));
        assert_eq!(parse_color("ansi(244)"), Some(Color::Indexed(244)));
        assert_eq!(parse_color("red"), Some(Color::Red));
        assert_eq!(parse_color("reset"), Some(Color::Reset));
        assert_eq!(parse_color("nopecolor"), None);
    }

    #[test]
    fn parses_padding_forms() {
        assert_eq!(parse_padding("1"), Some([1, 1, 1, 1]));
        assert_eq!(parse_padding("1 2"), Some([1, 2, 1, 2]));
        assert_eq!(parse_padding("1 2 3"), Some([1, 2, 3, 2]));
        assert_eq!(parse_padding("1 2 3 4"), Some([1, 2, 3, 4]));
        assert_eq!(parse_padding("a"), None);
    }

    #[test]
    fn parses_display() {
        assert_eq!(parse_display("none"), Some(Display::None));
        assert_eq!(parse_display("block"), Some(Display::Block));
        assert_eq!(parse_display("inline"), Some(Display::Inline));
        assert_eq!(parse_display("flex"), None);
    }

    #[test]
    fn parses_border_shape_keywords() {
        assert_eq!(parse_border_shape("none"), Some(BorderShape::None));
        assert_eq!(parse_border_shape("solid"), Some(BorderShape::Plain));
        assert_eq!(parse_border_shape("plain"), Some(BorderShape::Plain));
        assert_eq!(parse_border_shape("rounded"), Some(BorderShape::Rounded));
        assert_eq!(parse_border_shape("double"), Some(BorderShape::Double));
        assert_eq!(parse_border_shape("thick"), Some(BorderShape::Thick));
        assert_eq!(parse_border_shape("dashed"), None);
    }

    #[test]
    fn parses_border_radius_into_shape() {
        assert_eq!(parse_border_radius("0"), Some(BorderShape::Plain));
        assert_eq!(parse_border_radius("1"), Some(BorderShape::Rounded));
        assert_eq!(parse_border_radius("12"), Some(BorderShape::Rounded));
        assert_eq!(parse_border_radius("nope"), None);
    }

    #[test]
    fn parses_border_shorthand_pairs() {
        assert_eq!(
            parse_border_shorthand("rounded #ffcc00"),
            (
                Some(BorderShape::Rounded),
                Some(Color::Rgb(0xff, 0xcc, 0x00))
            )
        );
        assert_eq!(
            parse_border_shorthand("solid"),
            (Some(BorderShape::Plain), None)
        );
        assert_eq!(
            parse_border_shorthand("none"),
            (Some(BorderShape::None), None)
        );
        assert_eq!(
            parse_border_shorthand("#abcdef double"),
            (
                Some(BorderShape::Double),
                Some(Color::Rgb(0xab, 0xcd, 0xef))
            )
        );
        assert_eq!(parse_border_shorthand("garbage"), (None, None));
    }
}
