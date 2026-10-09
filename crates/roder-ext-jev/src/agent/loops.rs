//! Run-wide caps on a run that goes round in circles.
//!
//! Upstream's stall rule ends a run after three executed actions in a row
//! that changed nothing ([`super::stalled`]). It misses the runs that keep
//! changing the page, or never record a step, and those used to go on to the
//! 60-action or 120-call budget or to the timeout, which never falls back.
//! Three caps close the gaps. Each ends the run `blocked`, the status a stall
//! ends it with, and the fallback takes it over as it does after a stall (see
//! `fallback::trigger`):
//!
//! - **The same pair four times** ([`PAIR_LIMIT`]). A menu opened and closed
//!   again changes the page at every step, so no step is a no-op. The fourth
//!   time Jev would choose the same control on a page with the same
//!   fingerprint, the run ends [`JevStopCause::Looped`] before acting. Three
//!   in a row with nothing changing are the stall rule's, which fires first.
//!   The fingerprint is compared as the page reports it, digits and all: a
//!   counter that rises with every click is progress (the corpus's
//!   `step_budget` is that), and masking the numbers would end it at the
//!   fourth click. So a page with a clock on it never repeats a pair, and is
//!   left to the budgets.
//! - **Three stale decisions in a row** ([`STALE_LIMIT`]). A decision goes
//!   stale when the page moved under it, and nothing is recorded, so the next
//!   decision is the same one on a page that moved again: an on-screen ticker
//!   in the form being filled does it at every look. When the page read
//!   after the third shows nothing new, the run ends
//!   [`JevStopCause::Unsettled`]. Nothing new means the page is the same but
//!   for its digits ([`settled_view`]), the one place numbers are masked:
//!   they are all a ticker changes. A stale decision that leaves a different
//!   page is progress and starts the count again, and so does any step
//!   recorded in between.
//! - **Six waits in a row that changed nothing** ([`WAIT_LIMIT`]). Such a
//!   wait is also invisible to the stall rule: it neither counts as a no-op
//!   nor breaks a run of them, so `click, wait, click, wait, click` with
//!   nothing changing is a stall, and a model that only waits ends
//!   [`JevStopCause::Looped`].
//!
//! The page's own fingerprint (`page_changed`, and the one upstream hashes)
//! is not touched by any of this.

use std::collections::HashMap;

use serde_json::{Value, json};

use super::repeat::{CONTEXT_CHARS, LABEL_CHARS, where_it_sat};
use super::{Agent, chosen};
use crate::engine::{JevStatus, JevStop, JevStopCause};
use crate::session::cut;

/// The run ends instead of carrying out the same pair for this many times.
pub(super) const PAIR_LIMIT: usize = 4;
/// This many stale decisions in a row, each with nothing new on the page,
/// end the run.
pub(super) const STALE_LIMIT: usize = 3;
/// This many waits in a row that changed nothing end the run.
pub(super) const WAIT_LIMIT: usize = 6;
/// The last stale message in a reason is Jev's own wording; it is kept to a
/// line and cut like the other reasons.
const MESSAGE_CHARS: usize = 200;

/// A page's fingerprint and the control chosen on it.
pub(super) type Pair = (String, String);

/// What the caps remember across the run.
#[derive(Default)]
pub(super) struct LoopWatch {
    /// How many times each pair has been carried out.
    pairs: HashMap<Pair, usize>,
    stale: StaleRun,
}

/// The stale decisions in a row that left nothing new on the page.
#[derive(Default)]
struct StaleRun {
    count: usize,
    /// How many steps the history held at the last of them: a longer history
    /// means a step was recorded since, and the count starts again.
    steps: usize,
}

/// The pair `action` makes with the page it is chosen on. A wait makes none
/// (it has its own cap), and neither does a page without a fingerprint.
pub(super) fn pair_of(observation: &Value, action: &Value) -> Option<Pair> {
    if action["kind"].as_str() == Some("wait") {
        return None;
    }
    Some((
        observation["fingerprint"].as_str()?.to_string(),
        action["id"].as_str()?.to_string(),
    ))
}

/// Whether a history entry is a wait after which the page was the same.
pub(super) fn idle_wait(entry: &Value) -> bool {
    entry["kind"].as_str() == Some("wait") && entry["page_changed"] == json!(false)
}

/// How many of the latest chosen actions are idle waits, back to the first
/// that is not one.
fn idle_waits(history: &[Value]) -> usize {
    history
        .iter()
        .rev()
        .filter(chosen)
        .take_while(|entry| idle_wait(entry))
        .count()
}

impl LoopWatch {
    /// Count a pair that was carried out.
    pub(super) fn record(&mut self, pair: Option<Pair>) {
        if let Some(pair) = pair {
            *self.pairs.entry(pair).or_default() += 1;
        }
    }
}

impl Agent {
    /// Before `action` is carried out on the page it was chosen on: the stop
    /// to end the run with, when it would be the [`PAIR_LIMIT`]th time.
    pub(super) fn pair_looped(
        &mut self,
        action: &Value,
        pair: Option<&Pair>,
    ) -> Option<anyhow::Error> {
        let before = *self.watch.pairs.get(pair?)?;
        if before + 1 < PAIR_LIMIT {
            return None;
        }
        let label = self
            .tidy(&action["label"], LABEL_CHARS)
            .or_else(|| action["id"].as_str().map(str::to_string))
            .unwrap_or_default();
        let place = self
            .tidy(&where_it_sat(action), CONTEXT_CHARS)
            .map(|place| format!(" (in: {place})"))
            .unwrap_or_default();
        Some(self.circling(
            JevStopCause::Looped,
            format!(
                "Jev had chosen \"{label}\"{place} {before} times on a page that looked the \
                 same each time and was about to do it again. It stopped instead of going round \
                 in circles."
            ),
        ))
    }

    /// After a step is recorded and the page read again: the stop to end the
    /// run with, when [`WAIT_LIMIT`] waits in a row left the page as it was.
    pub(super) fn waited_out(&mut self) -> Option<anyhow::Error> {
        (idle_waits(&self.history) >= WAIT_LIMIT).then(|| {
            self.circling(
                JevStopCause::Looped,
                format!("Jev waited {WAIT_LIMIT} times in a row and the page did not change."),
            )
        })
    }

    /// After a decision went stale and the page was read again: the stop to
    /// end the run with, when it is the [`STALE_LIMIT`]th in a row that left
    /// nothing new on the page. `no_progress` is whether the page read again
    /// is the one the decision was made on, but for its digits.
    pub(super) fn after_stale(
        &mut self,
        stale: &anyhow::Error,
        no_progress: bool,
    ) -> Option<anyhow::Error> {
        let steps = self.history.len();
        let run = &mut self.watch.stale;
        if !no_progress {
            *run = StaleRun::default();
            return None;
        }
        if run.steps != steps {
            run.count = 0;
        }
        run.steps = steps;
        run.count += 1;
        if run.count < STALE_LIMIT {
            return None;
        }
        let message = stale.to_string();
        let message = message.split_whitespace().collect::<Vec<_>>().join(" ");
        Some(self.circling(
            JevStopCause::Unsettled,
            format!(
                "The page kept changing under Jev's decisions: {STALE_LIMIT} decisions in a row \
                 went stale and the page showed nothing new each time. The last: {}",
                cut(&message, MESSAGE_CHARS)
            ),
        ))
    }

    /// End the run `blocked` for `cause`; the loop's failure path stops it.
    fn circling(&mut self, cause: JevStopCause, reason: String) -> anyhow::Error {
        self.cause = Some(cause);
        JevStop::new(JevStatus::Blocked, reason).into()
    }
}

/// The page as the stale-decision cap reads it: its address and scroll as
/// they are, and its text and the visible controls' labels and values with
/// every run of digits made one `#`. A clock, a countdown or a "3 minutes
/// ago" changes only digits, so two looks that differ in nothing else are the
/// same page. Offscreen controls and the `context` and `section` naming each
/// one are left out, as the fingerprint leaves them.
pub(super) fn settled_view(observation: &Value) -> String {
    let mut view = String::new();
    for key in ["url", "scroll"] {
        view.push_str(&observation[key].to_string());
        view.push('\n');
    }
    mask_digits(observation["text"].as_str().unwrap_or_default(), &mut view);
    let actions = observation["actions"].as_array().into_iter().flatten();
    for action in actions.filter(|action| action["offscreen"] != json!(true)) {
        view.push('\n');
        for key in ["id", "kind", "checked", "selected", "expanded", "scrolled"] {
            view.push_str(&action[key].to_string());
            view.push('|');
        }
        for key in ["label", "value", "current_value"] {
            mask_digits(action[key].as_str().unwrap_or_default(), &mut view);
            view.push('|');
        }
    }
    for frame in observation["frames"].as_array().into_iter().flatten() {
        view.push('\n');
        view.push_str(frame["origin"].as_str().unwrap_or_default());
    }
    view
}

/// `text` onto `out`, each run of digits as a single `#`.
fn mask_digits(text: &str, out: &mut String) {
    let mut digits = false;
    for c in text.chars() {
        match (c.is_numeric(), digits) {
            (true, true) => {}
            (true, false) => {
                out.push('#');
                digits = true;
            }
            (false, _) => {
                out.push(c);
                digits = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(text: &str, label: &str) -> Value {
        json!({
            "url": "https://a.test/", "text": text, "scroll": {"y": 0},
            "actions": [{"id": "e1", "kind": "click", "label": label, "value": ""}],
        })
    }

    #[test]
    fn digits_are_masked_in_text_and_labels_and_nowhere_else() {
        let ticking = |n: u32| page(&format!("Session ends in {n}s"), &format!("Renew ({n}s)"));
        assert_eq!(settled_view(&ticking(59)), settled_view(&ticking(1203)));
        // A word is progress, and so is a different address or scroll.
        assert_ne!(
            settled_view(&page("Session ended", "Renew")),
            settled_view(&ticking(59))
        );
        let mut moved = ticking(59);
        moved["url"] = json!("https://a.test/2");
        assert_ne!(settled_view(&moved), settled_view(&ticking(59)));
        let mut scrolled = ticking(59);
        scrolled["scroll"] = json!({"y": 560});
        assert_ne!(settled_view(&scrolled), settled_view(&ticking(59)));
        // Controls count by id and kind as they are.
        let mut renamed = ticking(59);
        renamed["actions"][0]["id"] = json!("e2");
        assert_ne!(settled_view(&renamed), settled_view(&ticking(59)));
    }

    #[test]
    fn controls_below_the_fold_and_their_context_are_left_out() {
        let mut with_extras = page("Report", "Retry");
        let plain = settled_view(&with_extras);
        with_extras["actions"][0]["context"] = json!("Card 1");
        with_extras["actions"][0]["section"] = json!("Summary");
        with_extras["actions"].as_array_mut().unwrap().push(
            json!({"id": "e2", "kind": "click", "label": "Sale ends in 59s", "offscreen": true}),
        );
        assert_eq!(settled_view(&with_extras), plain);
    }

    #[test]
    fn a_wait_makes_no_pair_and_a_page_without_a_fingerprint_none() {
        let observation = json!({"fingerprint": "f"});
        let click = json!({"id": "e1", "kind": "click"});
        assert_eq!(
            pair_of(&observation, &click),
            Some(("f".to_string(), "e1".to_string()))
        );
        assert_eq!(
            pair_of(&observation, &json!({"id": "wait", "kind": "wait"})),
            None
        );
        assert_eq!(pair_of(&json!({}), &click), None);
    }

    #[test]
    fn only_idle_waits_at_the_end_of_the_history_count() {
        let wait = |changed: bool| json!({"kind": "wait", "page_changed": changed});
        let click = json!({"kind": "click", "page_changed": false});
        let banner = json!({"kind": super::super::COOKIE_BANNER, "page_changed": null});
        assert_eq!(idle_waits(&[]), 0);
        assert_eq!(idle_waits(&[wait(false), wait(false)]), 2);
        assert_eq!(idle_waits(&[wait(false), wait(false), click.clone()]), 0);
        assert_eq!(idle_waits(&[click, wait(false), banner, wait(false)]), 2);
        // A wait that changed the page is progress, not idle.
        assert_eq!(idle_waits(&[wait(false), wait(true), wait(false)]), 1);
    }
}
