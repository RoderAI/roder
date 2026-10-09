//! Asking the decision again when the service's reply cannot be used.
//!
//! The service answered, and was probably billed, but what it sent cannot be
//! used: an action the page never offered, probabilities that do not add up,
//! a refusal, or a body that cannot be decoded at all (after the transport's
//! one resend). Live runs that ended on one passed on a rerun, so the loop
//! asks the same decision again, at most [`ASKS_AGAIN`] times, before it
//! gives up. A usable reply starts the count again, so replies that go wrong
//! now and then never add up to a stop. Every unusable reply still counts in
//! the run's usage and model calls (see `predict`), and the model-call
//! budget still ends a run that keeps asking. A reply that failed validation
//! carries the usage the service reported; one that could not be decoded
//! carries none, so the run's token sums read `unknown` rather than a number
//! that was never reported.
//!
//! Giving up ends the run `error` with [`JevStopCause::DecisionUnusable`]
//! and says how many replies there were and why the first was unusable. This
//! is the one `error` that falls back (see `fallback::trigger`): a frontier
//! model with the full browser tools goes on from there.

use super::*;
use crate::session::cut;
use crate::usage::UnusableAnswer;

/// How many times one decision is asked again after an unusable reply.
pub(super) const ASKS_AGAIN: usize = 2;

/// The first reason is service text; it is kept to a line and cut.
const REASON_CHARS: usize = 200;

/// The unusable replies to the decision being asked, in a row.
#[derive(Default)]
pub(super) struct UnusableStreak {
    replies: usize,
    first_reason: Option<String>,
}

impl UnusableStreak {
    /// A usable reply: the next unusable one is the first again.
    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }
}

impl Agent {
    /// Whether `error` is a billed reply the loop may ask again about.
    pub(super) fn is_unusable_reply(error: &anyhow::Error) -> bool {
        UnusableAnswer::is_behind(error)
    }

    /// Note an unusable reply. `None` means ask the decision again; `Some`
    /// is why the run ends, now stopped `error` with its cause set.
    pub(super) fn after_unusable_reply(&mut self, error: &anyhow::Error) -> Option<String> {
        let streak = &mut self.unusable_streak;
        streak.replies += 1;
        let first = streak.first_reason.get_or_insert_with(|| {
            let line = error.to_string();
            let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
            cut(&line, REASON_CHARS)
        });
        if streak.replies <= ASKS_AGAIN {
            return None;
        }
        let first = first.trim_end_matches('.');
        let end = if first.ends_with('…') { "" } else { "." };
        let reason = format!(
            "The decision service gave {} unusable replies in a row. The first: {first}{end}",
            streak.replies
        );
        self.stop(JevStatus::Error);
        self.cause = Some(JevStopCause::DecisionUnusable);
        Some(reason)
    }
}
