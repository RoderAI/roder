//! The repeat guard: an unsure click that repeats the click just made.
//!
//! Live runs show the model split between clicking again and DONE once the
//! page confirms a click (confidence 0.43 to 0.66), while a click that is
//! meant to repeat stays above 0.75. Clicking again is the costly mistake: a
//! second item in the cart, a second mail deleted. Below
//! [`REPEAT_CONFIDENCE`], right after a click that changed the page and on
//! the same label, the run ends `done` without clicking in two cases:
//!
//! - the same control again, and
//! - a twin: another control with that label, when the label names a
//!   commitment ([`crate::irreversible::names_commitment`]). The recorded
//!   `icon_by_picture` failure deleted Bob's budget mail at 0.99 and his
//!   lunch mail at 0.60: two trash icons, one label, two ids.
//!
//! A twin whose label commits nothing ("Open", "Next") is clicked as ever. A
//! goal that needs the twin too ("remove all Bob mails") is cut short when
//! the model is unsure; the result says which click was not made, and a new
//! call starts with no earlier click, so it can make it.

use serde_json::{Value, json};

use super::Agent;
use crate::engine::{JevDecision, JevSuppressedClick, JevSuppressedKind};
use crate::irreversible::names_commitment;
use crate::session::cut;

/// Below this, a repeat of the last effective click is read as DONE.
pub(crate) const REPEAT_CONFIDENCE: f64 = 0.7;
/// A label in the result is cut to this many characters.
pub(super) const LABEL_CHARS: usize = 80;
/// A row or section in the result is cut to this many characters.
pub(super) const CONTEXT_CHARS: usize = 120;

impl Agent {
    /// The click `action`, chosen with `decision`, is an unsure repeat of
    /// the click that just took effect: what to report in its place.
    pub(super) fn unsure_repeat(
        &self,
        action: &Value,
        decision: &JevDecision,
    ) -> Option<JevSuppressedClick> {
        let last = self.last_step()?;
        let repeats = action["kind"] == json!("click")
            && decision.confidence < REPEAT_CONFIDENCE
            && last["kind"] == action["kind"]
            && last["action"] == action["label"]
            && last["page_changed"] == json!(true);
        if !repeats {
            return None;
        }
        let kind = if last["choice"] == json!(decision.choice) {
            JevSuppressedKind::SameControl
        } else if names_commitment(action["label"].as_str().unwrap_or_default()) {
            JevSuppressedKind::TwinControl
        } else {
            return None;
        };
        Some(JevSuppressedClick {
            kind,
            label: self.tidy(&action["label"], LABEL_CHARS).unwrap_or_default(),
            context: self.tidy(&where_it_sat(action), CONTEXT_CHARS),
            previous_context: self.tidy(&last["context"], CONTEXT_CHARS),
            confidence: decision.confidence,
        })
    }

    /// Page text for the caller: typed secrets scrubbed, on one line, cut
    /// short; `None` when nothing is left.
    pub(super) fn tidy(&self, text: &Value, chars: usize) -> Option<String> {
        let scrubbed = self.secrets.scrub(text.as_str()?);
        let line = scrubbed.split_whitespace().collect::<Vec<_>>().join(" ");
        (!line.is_empty()).then(|| cut(&line, chars))
    }
}

/// The card, row or section an observed action sat in, as a history step
/// records it.
pub(super) fn where_it_sat(action: &Value) -> Value {
    action
        .get("context")
        .or_else(|| action.get("section"))
        .cloned()
        .unwrap_or(Value::Null)
}
