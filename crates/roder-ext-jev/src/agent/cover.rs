//! Naming what covered a target, for the step that found it covered.
//!
//! The browser reads the name off the page (act.js), so it is page text: it
//! can show a typed secret, run to any length and say anything, including
//! text meant for a model. Before it is recorded it is scrubbed of typed
//! secrets, then [`tidy_cover`] puts it on one line, blanks control
//! characters and the direction and zero-width format characters (so it
//! cannot reorder the text around it), replaces quote marks (so it cannot end
//! the quotes it is shown in) and cuts it to [`COVER_CHARS`]. The digest and
//! the fallback's brief show it inside their page-supplied sections,
//! labelled untrusted. It is never put in the request to the decision
//! service: `decide::request_body` names the history keys it sends, and
//! `covered_by` is not one of them.

use super::Agent;
use crate::engine::{COVER_CONTROLS, Covered};
use crate::session::cut;

/// A cover's name in the record, the digest and the brief is cut to this
/// many characters.
pub(crate) const COVER_CHARS: usize = 100;

/// Format characters that show nothing but change how the text around them is
/// drawn or read: the Arabic letter mark, the zero-width space and joiners,
/// the left-to-right and right-to-left marks, the embeddings and overrides,
/// the isolates and the byte-order mark. `char::is_control` does not cover
/// them.
fn is_direction_or_zero_width(c: char) -> bool {
    matches!(
        c,
        '\u{061c}'
            | '\u{200b}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
            | '\u{feff}'
    )
}

/// A cover's name as it is shown: on one line, without control characters,
/// direction overrides or double quotes, cut to [`COVER_CHARS`]. `None` when
/// nothing is left. Those characters are blanked before the words are
/// split, so they separate words as whitespace does and never join them.
pub(crate) fn tidy_cover(name: &str) -> Option<String> {
    let blanked = name
        .chars()
        .map(|c| {
            if c.is_control() || is_direction_or_zero_width(c) {
                ' '
            } else {
                c
            }
        })
        .collect::<String>();
    let line = blanked
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('"', "'");
    let line = cut(&line, COVER_CHARS);
    (!line.is_empty()).then_some(line)
}

impl Agent {
    /// The name `error` gives what covered the target, scrubbed and tidied
    /// for the record; `None` when it names nothing. Scrubbed before it is
    /// cut, so a secret is never cut in half and left behind.
    pub(super) fn cover_name(&self, error: &Covered) -> Option<String> {
        tidy_cover(&self.secrets.scrub(error.cover()?))
    }

    /// The controls `error` says what covered the target offers, each
    /// scrubbed and tidied as a name is, then cut to [`CONTROL_CHARS`].
    pub(super) fn cover_controls(&self, error: &Covered) -> Vec<String> {
        error
            .controls()
            .iter()
            .filter_map(|control| tidy_control(&self.secrets.scrub(control)))
            .collect()
    }
}

/// Each control a cover offers is cut to this many characters.
pub(crate) const CONTROL_CHARS: usize = 60;

/// A cover's control as it is shown: tidied as [`tidy_cover`] tidies a name,
/// and cut to [`CONTROL_CHARS`].
pub(crate) fn tidy_control(name: &str) -> Option<String> {
    tidy_cover(name).map(|name| cut(&name, CONTROL_CHARS))
}

/// `, which offers "Close", "No thanks"`: what a cover offers to press, as
/// the digest and the fallback's brief say it after its name, each control
/// tidied and at most [`COVER_CONTROLS`] of them; empty when it offers none.
pub(crate) fn offers<'a>(controls: impl IntoIterator<Item = &'a str>) -> String {
    let controls = controls
        .into_iter()
        .filter_map(tidy_control)
        .take(COVER_CONTROLS)
        .map(|control| format!("\"{control}\""))
        .collect::<Vec<_>>();
    match controls.is_empty() {
        true => String::new(),
        false => format!(", which offers {}", controls.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_put_on_one_line_without_quotes_or_control_characters() {
        assert_eq!(
            tidy_cover("  Say \"hi\"\n\tto\u{7}  us "),
            Some("Say 'hi' to us".to_string())
        );
        for nothing in ["", "   ", "\u{7}\u{1b}", "\n"] {
            assert_eq!(tidy_cover(nothing), None, "{nothing:?}");
        }
    }

    #[test]
    fn direction_overrides_and_zero_width_characters_are_not_kept() {
        // A right-to-left override would reorder what follows it in a viewer
        // that honours direction controls, inside the quotes it is shown in.
        assert_eq!(
            tidy_cover("Cancel \u{202e}esoprus\u{2069}"),
            Some("Cancel esoprus".to_string())
        );
        for format in [
            '\u{061c}', '\u{200b}', '\u{200e}', '\u{200f}', '\u{202a}', '\u{202d}', '\u{2066}',
            '\u{2069}', '\u{feff}',
        ] {
            assert_eq!(
                tidy_cover(&format!("Close{format} dialog")),
                Some("Close dialog".to_string()),
                "{format:?}"
            );
        }
        // A name of nothing else is no name.
        assert_eq!(tidy_cover("\u{202e}\u{200b} \u{2066}\u{feff}"), None);
    }

    #[test]
    fn a_control_is_tidied_and_cut_shorter_than_a_name() {
        assert_eq!(
            tidy_control("No \"thanks\",\n take me\u{202e} on"),
            Some("No 'thanks', take me on".to_string())
        );
        let long = tidy_control(&"word ".repeat(40)).unwrap();
        assert_eq!(long.chars().count(), CONTROL_CHARS);
        assert_eq!(tidy_control(" \u{200b} "), None);
    }

    #[test]
    fn a_name_is_cut_to_the_cap() {
        let name = tidy_cover(&"abc ".repeat(100)).unwrap();
        assert_eq!(name.chars().count(), COVER_CHARS);
        assert!(name.ends_with('…'));
        let exact = "z".repeat(COVER_CHARS);
        assert_eq!(tidy_cover(&exact), Some(exact));
    }
}
