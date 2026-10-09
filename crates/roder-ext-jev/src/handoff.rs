//! Handoffs are outcomes, not failed tool calls.
//!
//! A run that ends `needs_input`, `needs_confirmation` or `access_denied`
//! has done what it should: it found something only the caller can settle (a
//! value the goal lacks, the user's say-so for an action that cannot be
//! undone, a site that refused automated access) and said so, with nothing
//! dispatched. Roder's runtime counts every tool result with `is_error` set
//! toward its stop after five in a row, so reporting each of these as an
//! error made a caller that was working through legitimate handoffs (a
//! missing field, then a confirmation, then another site's refusal) look like
//! one that was failing.
//!
//! So a first handoff is not an error, and its data says so with
//! `outcome_class: "handoff"`. The same handoff coming back unchanged (the
//! same status, on the same page, for the same reason) means the caller did
//! not act on it, and that result is an error again, with
//! `outcome_class: "repeated_handoff"`. Every other status keeps its old
//! meaning and gets no `outcome_class`: only `done` and `closed` are not
//! errors. The text the caller reads is the same either way.
//!
//! What a handoff said is remembered per session, and so per thread: see
//! [`SessionState::classify`](crate::session::SessionState::classify).

use serde_json::{Value, json};

use crate::session::cut;

#[cfg(test)]
mod tests;

/// A handoff that has not come back unchanged.
pub(crate) const HANDOFF: &str = "handoff";
/// A handoff identical to the session's last call's.
pub(crate) const REPEATED_HANDOFF: &str = "repeated_handoff";

/// The statuses that hand a decision to the caller.
const STATUSES: [&str; 3] = ["needs_input", "needs_confirmation", "access_denied"];

/// Longest a remembered reason or address is, in characters. Both sides of a
/// comparison are cut the same way.
const REASON_CHARS: usize = 300;
const URL_CHARS: usize = 2048;

/// What a handoff said: enough to tell whether the next one is the same.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Handoff {
    status: String,
    /// The page the call ended on.
    url: String,
    /// Why it stopped, which names the field, the control or the evidence.
    reason: String,
}

impl Handoff {
    /// The handoff `data`, a finished call's result, reports, if it does.
    fn of(data: &Value) -> Option<Self> {
        let status = data["status"].as_str().filter(|s| STATUSES.contains(s))?;
        let reason = data["stopped_because"]
            .as_str()
            .unwrap_or_default()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        Some(Self {
            status: status.to_string(),
            url: cut(data["url"].as_str().unwrap_or_default(), URL_CHARS),
            reason: cut(&reason, REASON_CHARS),
        })
    }
}

/// Mark `data` as a handoff or a repeat of the one `last` holds, and keep
/// what it said as the new `last`. A result that is not a handoff forgets
/// it: only an unbroken run of the same handoff is a repeat, as only an
/// unbroken run of errors adds up toward Roder's stop.
pub(crate) fn classify(last: &mut Option<Handoff>, data: &mut Value) {
    let current = Handoff::of(data);
    let class = match (&current, &*last) {
        (None, _) => None,
        (Some(now), Some(before)) if now == before => Some(REPEATED_HANDOFF),
        (Some(_), _) => Some(HANDOFF),
    };
    if let Some(class) = class {
        data["outcome_class"] = json!(class);
    }
    *last = current;
}

/// Whether `data`, a finished call's result, is a failed tool call. A
/// handoff status is one unless the session marked it a first handoff; an
/// unmarked one is not trusted.
pub(crate) fn is_error(data: &Value) -> bool {
    match data["status"].as_str() {
        Some("done" | "closed") => false,
        Some(status) if STATUSES.contains(&status) => {
            data["outcome_class"].as_str() != Some(HANDOFF)
        }
        _ => true,
    }
}
