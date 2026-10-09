//! Naming what covered a target, for the step that found it covered.
//!
//! The browser reads the name off the page (act.js), so it is page text: it
//! can show a typed secret, run to any length and say anything, including
//! text meant for a model. Before it is recorded it is scrubbed of typed
//! secrets, then [`tidy_cover`] puts it on one line, strips control
//! characters and quote marks (so it cannot end the quotes it is shown in)
//! and cuts it to [`COVER_CHARS`]. The digest and the fallback's brief show
//! it inside their page-supplied sections, labelled untrusted. It is never
//! put in the request to the decision service: `decide::request_body` names
//! the history keys it sends, and `covered_by` is not one of them.

use super::Agent;
use crate::engine::Covered;
use crate::session::cut;

/// A cover's name in the record, the digest and the brief is cut to this
/// many characters.
pub(crate) const COVER_CHARS: usize = 100;

/// A cover's name as it is shown: on one line, without control characters
/// or double quotes, cut to [`COVER_CHARS`]. `None` when nothing is left.
pub(crate) fn tidy_cover(name: &str) -> Option<String> {
    let line = name
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| if c == '"' { '\'' } else { c })
        .collect::<String>();
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
    fn a_name_is_cut_to_the_cap() {
        let name = tidy_cover(&"abc ".repeat(100)).unwrap();
        assert_eq!(name.chars().count(), COVER_CHARS);
        assert!(name.ends_with('…'));
        let exact = "z".repeat(COVER_CHARS);
        assert_eq!(tidy_cover(&exact), Some(exact));
    }
}
