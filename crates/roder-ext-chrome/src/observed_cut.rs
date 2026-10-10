//! What an observation cuts off, and what it says about it.
//!
//! Three things about a page are only kept in part: the role a control is
//! listed under (a page supplies it, and a line must not grow with it), the
//! address (kept long enough to tell two pages apart, shown shorter), and the
//! text (the observer sends the start of it). All three come from the page and
//! are untrusted.

use crate::observed::one_line;

/// Characters of a control's role a line carries. A real role is one short
/// word; anything longer is the page's own words.
const ROLE_CHARS: usize = 40;
/// Characters of an address an observation keeps to tell one page from
/// another: far more than a result shows (see `observed_render`), and bounded
/// so that a `data:` address is not held whole.
pub(crate) const URL_KEEP_CHARS: usize = 4_096;
/// The most the extension keeps of a page's text, in the UTF-16 units that
/// JavaScript's `slice` counts.
const EXTENSION_TEXT_UNITS: usize = 12_000;

/// Said instead of "No visible change." when the text that was compared was
/// only the start of the page's: the later part was not looked at.
pub(crate) const NO_CHANGE_IN_THE_START: &str = "No visible change in the controls or in the start of the page text (the text is cut, so later changes are not compared).";

/// The role a control is listed under: the first of `names` that is anything
/// once it is put on one line and cut (a role, else a type, else a tag).
pub(crate) fn role_of<'a>(names: impl IntoIterator<Item = Option<&'a str>>) -> String {
    names
        .into_iter()
        .flatten()
        .map(|name| one_line(name, ROLE_CHARS))
        .find(|name| !name.is_empty())
        .unwrap_or_default()
}

/// Whether the extension cut `text`, as it sent it: it keeps at most
/// [`EXTENSION_TEXT_UNITS`] and sends exactly that many when the page had more.
/// The text is measured as sent, before it is made one line: that step turns
/// characters into spaces and spaces into one, which would hide a cut.
pub(crate) fn cut_by_extension(text: &str) -> bool {
    text.encode_utf16().count() >= EXTENSION_TEXT_UNITS
}

#[cfg(test)]
#[path = "observed_cut_tests.rs"]
mod tests;
