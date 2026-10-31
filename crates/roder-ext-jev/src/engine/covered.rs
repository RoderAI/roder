//! The refusal of a target that another element covers.

/// The target is still observed as it was, but another element would take
/// the click at every point tried, so no input was dispatched. Unlike a stale
/// observation this is recorded as an executed step that changed nothing, so
/// a target that stays covered ends the run through the stall rule.
///
/// The message is fixed text. What covers the target is named apart from it
/// ([`Covered::cover`]), because that name is the page's own text: the loop
/// scrubs it of typed secrets, puts it on one line and cuts it before it is
/// recorded, and the message goes nowhere a name would need that.
#[derive(Debug)]
pub struct Covered {
    message: String,
    cover: Option<String>,
    controls: Vec<String>,
}

/// At most this many of a cover's controls are named.
pub const COVER_CONTROLS: usize = 8;

impl Covered {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            cover: None,
            controls: Vec::new(),
        }
    }

    /// Name the controls what covers the target offers (its close button,
    /// an odd "No thanks, take me to the site"), as the page names them.
    /// Blank names are dropped, and at most [`COVER_CONTROLS`] are kept.
    pub fn with_controls(mut self, controls: impl IntoIterator<Item = String>) -> Self {
        self.controls = controls
            .into_iter()
            .filter(|control| !control.trim().is_empty())
            .take(COVER_CONTROLS)
            .collect();
        self
    }

    /// The controls what covers the target offers. Page text, untrusted and
    /// not yet scrubbed.
    pub fn controls(&self) -> &[String] {
        &self.controls
    }

    /// Name what covers the target, as the page names it: its label, its
    /// text, or its tag. A blank name says nothing and is dropped.
    pub fn with_cover(mut self, cover: impl Into<String>) -> Self {
        let cover = cover.into();
        self.cover = (!cover.trim().is_empty()).then_some(cover);
        self
    }

    /// What covers the target, when the browser could name it. Page text,
    /// untrusted and not yet scrubbed.
    pub fn cover(&self) -> Option<&str> {
        self.cover.as_deref()
    }
}

impl std::fmt::Display for Covered {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.message)
    }
}

impl std::error::Error for Covered {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_is_kept_apart_from_the_fixed_message() {
        let covered = Covered::new("Another element covers the target.").with_cover("Sale popup");
        assert_eq!(covered.cover(), Some("Sale popup"));
        assert_eq!(covered.to_string(), "Another element covers the target.");
        assert_eq!(Covered::new("x").cover(), None);
    }

    #[test]
    fn controls_are_kept_without_blanks_and_capped() {
        let names = ["Close", " ", "No thanks"]
            .into_iter()
            .chain(["x"; 10])
            .map(str::to_string);
        let covered = Covered::new("x").with_controls(names);
        assert_eq!(covered.controls().len(), COVER_CONTROLS);
        assert_eq!(covered.controls()[..2], ["Close", "No thanks"]);
        assert!(Covered::new("x").controls().is_empty());
    }

    #[test]
    fn a_blank_name_names_nothing() {
        for blank in ["", "  ", "\n\t"] {
            assert_eq!(Covered::new("x").with_cover(blank).cover(), None);
        }
    }
}
