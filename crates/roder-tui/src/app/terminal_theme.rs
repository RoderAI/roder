use super::{Color, TerminalColorDepth, Theme};

pub(super) fn detect_dark_background() -> Option<bool> {
    dark_background_from_colorfgbg(std::env::var("COLORFGBG").ok().as_deref())
}

fn dark_background_from_colorfgbg(value: Option<&str>) -> Option<bool> {
    let background = value?.rsplit(';').next()?.parse::<u8>().ok()?;
    match background {
        0..=6 | 8 => Some(true),
        7 | 9..=15 => Some(false),
        // COLORFGBG conventionally reports ANSI colors, not palette indexes.
        _ => None,
    }
}

impl Theme {
    pub(super) fn for_terminal_background(
        dark: Option<bool>,
        color_depth: TerminalColorDepth,
    ) -> Self {
        let mut theme = Self::for_terminal_capabilities(dark.unwrap_or(true), color_depth);
        // Unknown does not mean dark: keep terminal-default text on its native
        // surface rather than pairing a light terminal foreground with dark fill.
        if dark.is_none() {
            theme.user_message_bg = Color::Reset;
        }
        theme
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ThemeOverrides;

    #[test]
    fn colorfgbg_preserves_unknown_backgrounds() {
        for value in [None, Some(""), Some("invalid"), Some("15;"), Some("0;256")] {
            assert_eq!(dark_background_from_colorfgbg(value), None);
        }
        assert_eq!(dark_background_from_colorfgbg(Some("15;0")), Some(true));
        assert_eq!(dark_background_from_colorfgbg(Some("0;15")), Some(false));
        assert_eq!(
            dark_background_from_colorfgbg(Some("0;default;7")),
            Some(false)
        );
    }

    #[test]
    fn unknown_terminal_keeps_prompt_and_rail_on_native_background() {
        let default_css = ThemeOverrides::from_css(include_str!("../../../../themes/default.css"))
            .expect("default theme parses");
        for depth in [TerminalColorDepth::Ansi256, TerminalColorDepth::TrueColor] {
            let baseline = Theme::for_terminal_background(None, depth);
            for theme in [baseline, baseline.apply_overrides(&default_css)] {
                assert_eq!(theme.user_surface().fg, Some(Color::Reset));
                assert_eq!(theme.user_surface().bg, Some(Color::Reset));
                assert_eq!(theme.user_rail().bg, Some(Color::Reset));
            }
        }
    }

    #[test]
    fn known_terminal_keeps_light_and_dark_prompt_shading() {
        for dark in [false, true] {
            for depth in [TerminalColorDepth::Ansi256, TerminalColorDepth::TrueColor] {
                assert_eq!(
                    Theme::for_terminal_background(Some(dark), depth).user_message_bg,
                    super::super::user_message_bg_for(dark, depth)
                );
            }
        }
    }

    #[test]
    fn light_css_overrides_dark_terminal_prompt_and_rail() {
        let css = ThemeOverrides::from_css(include_str!("../../../../themes/light.css"))
            .expect("light theme parses");
        let theme = Theme::for_terminal_background(Some(true), TerminalColorDepth::TrueColor)
            .apply_overrides(&css);
        let light_surface = Some(Color::Rgb(0xf0, 0xee, 0xe6));
        assert_eq!(theme.user_surface().bg, light_surface);
        assert_eq!(theme.user_rail().bg, light_surface);
        assert_eq!(theme.user_surface().fg, Some(Color::Rgb(0x1f, 0x1f, 0x1f)));
    }

    #[test]
    fn explicit_prompt_background_still_wins_on_unknown_terminals() {
        let css = ThemeOverrides::from_css(":root { --user-message-bg: #123456; }")
            .expect("theme parses");
        let theme = Theme::for_terminal_background(None, TerminalColorDepth::TrueColor)
            .apply_overrides(&css);
        assert_eq!(theme.user_surface().bg, Some(Color::Rgb(0x12, 0x34, 0x56)));
    }
}
