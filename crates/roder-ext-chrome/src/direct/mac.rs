//! Mac pages and the editing chords a model sends as it would on Windows.
//!
//! A computer-use model presses Control+a to select all. A Mac page does
//! nothing with that chord: only Command chords carry the editing commands,
//! so the next typed text lands after the old text instead of replacing it.
//! In an editable field of a Mac page the Control chords of the editing
//! family are sent as their Command twins, and the result says so.

use serde_json::json;

use super::keys::Chord;
use super::look::helper;
use super::session::DirectSession;

impl DirectSession {
    /// Whether the page reports a Mac platform.
    pub(crate) async fn mac_page(&mut self) -> anyhow::Result<bool> {
        Ok(self
            .client
            .evaluate_isolated("navigator.platform")
            .await?
            .as_str()
            .is_some_and(|platform| platform.contains("Mac")))
    }

    /// The chord to send in place of `raw`, and what to tell the model about
    /// it, when `raw` is a Control editing chord, the page is a Mac page and
    /// focus is in an editable field. `None` leaves the chord as it was asked.
    pub(crate) async fn mac_chord(
        &mut self,
        raw: &str,
    ) -> anyhow::Result<Option<(String, String)>> {
        let Ok(chord) = Chord::parse(raw) else {
            return Ok(None);
        };
        let Some(twin) = chord.mac_equivalent() else {
            return Ok(None);
        };
        if !self.mac_page().await?
            || helper(&mut self.client, "editable(null)").await? != json!(true)
        {
            return Ok(None);
        }
        let said = format!(
            "{} was sent as {} on macOS (Control chords do not edit text there)",
            chord.spoken(),
            twin.spoken()
        );
        Ok(Some((twin.name(), said)))
    }
}
