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

use std::hash::{DefaultHasher, Hash, Hasher};

use serde_json::{Value, json};

#[cfg(test)]
mod tests;

/// A handoff that has not come back unchanged.
pub(crate) const HANDOFF: &str = "handoff";
/// A handoff identical to the session's last call's.
pub(crate) const REPEATED_HANDOFF: &str = "repeated_handoff";

/// The statuses that hand a decision to the caller.
const STATUSES: [&str; 3] = ["needs_input", "needs_confirmation", "access_denied"];

/// What a handoff said: enough to tell whether the next one is the same.
///
/// The address and the reason are kept as fingerprints of their whole text,
/// whitespace aside, so nothing is cut before they are compared (a long
/// reason that differs only at its end is another handoff) and a remembered
/// handoff stays a few bytes. A fingerprint is only ever compared with
/// another from the same process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Handoff {
    status: String,
    /// The page the call ended on.
    url: u64,
    /// Why it stopped, which names the field, the control or the evidence.
    reason: u64,
}

/// A fingerprint of `text`'s words: runs of whitespace and whitespace at
/// either end make no difference, and each word ends where it ends.
fn fingerprint(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    for word in text.split_whitespace() {
        word.hash(&mut hasher);
    }
    hasher.finish()
}

impl Handoff {
    /// The handoff `data`, a finished call's result, reports, if it does.
    fn of(data: &Value) -> Option<Self> {
        let status = data["status"].as_str().filter(|s| STATUSES.contains(s))?;
        Some(Self {
            status: status.to_string(),
            url: fingerprint(data["url"].as_str().unwrap_or_default()),
            reason: fingerprint(data["stopped_because"].as_str().unwrap_or_default()),
        })
    }
}

/// Mark `data` as a handoff or a repeat of the one `last` holds, and keep
/// what it said as the new `last`. A finished `jev_browse` result that is
/// not a handoff forgets it: only an unbroken run of the same handoff is a
/// repeat, as only an unbroken run of errors adds up toward Roder's stop. A
/// call that never gets a result to classify (it ended in an error, found
/// the session busy or closed its tab) and a `jev_tab_*` call leave it as it
/// was, so the same handoff coming back after one is still a repeat.
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
