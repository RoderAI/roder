//! The irreversible-action gate: off by default.
//!
//! With the gate on ([`crate::JevEngineConfig::with_irreversible_gate`], or
//! `JEV_CONFIRM_IRREVERSIBLE=1` for the built-in tool), the decision request
//! carries one extra Noul question per offered action that may commit
//! something, in the same request: no extra round trip. The loop reads the
//! answer for the action the model chose and, when it says the action cannot
//! be undone, stops the run `needs_confirmation` instead of dispatching it,
//! unless the run is authorized and the decision is confident.
//!
//! Which actions are asked about is decided in code ([`shortlist`]): every
//! Enter (a form, a chat box and a comment box all send on it), every click
//! whose label or link names one of [`shortlist::COMMIT_WORDS`], then every click whose
//! heading or row names one, that agrees to a dialog ("Yes, I'm sure"), or
//! that posts its form, at most [`MAX_QUESTIONS`] of them, the stronger
//! evidence first. A playback control (play, pause, mute) is never asked
//! about. Each question quotes the label, where the control sits, where a
//! link goes and what form a submit sends ([`question`]). Whether one of
//! those commits is the model's judgment. An action whose answer is missing
//! or invalid is treated as irreversible: the gate fails closed. So is a
//! click whose label names a commitment, or an Enter, that the cap left
//! unasked; a click shortlisted only by its address, heading, row or form
//! that the cap left unasked is dispatched, as it was before the gate read
//! past the label, so twenty "Add to cart" forms that each post cannot stop
//! a run. fastbrowse (MIT) asks
//! about every click, in a second request; this asks about a shortlist, in
//! the same one.
//!
//! Two stops need no judgment of the action (fastbrowse's
//! `changes_credentials` and `starts_download`): a submit that sets a new
//! password stops unless the run is authorized and confident, and is not
//! asked about; and a click the page says saves a file stops whenever the
//! goal rules out downloading, which one more Noul asks of the goal alone
//! when such a click is on offer. Authorization does not lift that one: it
//! lets a run commit what the goal asks for, never what it forbids.
//!
//! The question and answer shapes (`"type": "noul"`, a string `instructions`,
//! optional `criteria.true`/`false`, the answer's `noul` P(yes)) follow
//! fastbrowse's direct-API client, and the thresholds below are fastbrowse's,
//! not measured on Jev.

use serde_json::Value;

use crate::space::ActionSpace;

mod question;
pub(crate) mod shortlist;

pub(crate) use shortlist::names_commitment;
use shortlist::{Tier, changes_credentials, starts_download, tier};

/// Above this P(irreversible), the chosen action is not dispatched unless
/// the run is authorized (fastbrowse's `irreversible_above`). The same line
/// decides whether the goal rules out downloading.
pub(crate) const IRREVERSIBLE_ABOVE: f64 = 0.5;
/// An authorized run dispatches an irreversible action only at or above
/// this call confidence (fastbrowse's `sensitive_act_from`).
pub(crate) const AUTHORIZED_CONFIDENCE: f64 = 0.90;
/// At most this many questions about actions per request.
pub(crate) const MAX_QUESTIONS: usize = 8;
/// The key of the question whether the goal rules out downloading.
pub(crate) const DOWNLOADS_KEY: &str = "downloads_forbidden";

/// Whether the gate asks about this observed action: an Enter, or a click
/// whose evidence says it may commit (see [`shortlist`]).
pub(crate) fn gated(action: &Value) -> bool {
    tier(action).is_some()
}

/// One question the gate added to a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Asked {
    /// The question's key in `questions`, and so in `answers`.
    pub(crate) key: String,
    /// `CLICK` or `PRESS_ENTER`, or `DOWNLOADS` for [`DOWNLOADS_KEY`].
    pub(crate) operation: String,
    /// The target index the question is about; empty for downloads.
    pub(crate) target: String,
}

/// The key for the question about `operation` on `target`.
fn key(operation: &str, target: &str) -> String {
    let short = if operation == "PRESS_ENTER" {
        "enter"
    } else {
        "click"
    };
    format!("irreversible_{short}_{target}")
}

/// Every observed action the space offers a target for.
fn offered(space: &ActionSpace) -> impl Iterator<Item = &Value> {
    space
        .targets
        .iter()
        .flat_map(|(_, targets)| targets.iter().map(|(_, action)| action))
}

/// The actions the gate asks about, in the order it asks: every Enter
/// target, then the clicks the shortlist names, stronger evidence first and
/// in request order within a tier, at most [`MAX_QUESTIONS`]. A target that
/// submits a new password is left out: it stops whatever the answer.
fn shortlisted(space: &ActionSpace) -> Vec<(&'static str, &String, &Value)> {
    let offered_in = |operation: &str| {
        space
            .targets_for(operation)
            .into_iter()
            .flatten()
            .filter(|(_, action)| !changes_credentials(offered(space), action))
            .filter_map(|(index, action)| Some((tier(action)?, index, action)))
            .collect::<Vec<_>>()
    };
    let mut clicks = offered_in("CLICK");
    // Stable: in request order within a tier.
    clicks.sort_by_key(|(tier, _, _)| *tier);
    offered_in("PRESS_ENTER")
        .into_iter()
        .map(|(_, target, action)| ("PRESS_ENTER", target, action))
        .chain(
            clicks
                .into_iter()
                .map(|(_, target, action)| ("CLICK", target, action)),
        )
        .take(MAX_QUESTIONS)
        .collect()
}

/// Whether the gate had room to ask about `action` among the `offered`
/// actions of its observation: the request is built from them alone, so
/// the shortlist is built again here.
fn had_room(action: &Value, offered: &[Value]) -> bool {
    let space = crate::space::action_space(offered);
    shortlisted(&space)
        .into_iter()
        .any(|(_, _, asked)| asked == action)
}

/// Add the gate's questions to a decision request built by
/// `decide::request_body` from `space`, after its own, and return what was
/// asked: every Enter target first, then the clicks the shortlist names,
/// stronger evidence first and in the order the request offers them, up to
/// [`MAX_QUESTIONS`]. A target that submits a new password is not asked
/// about: it stops whatever the answer. When a click on offer would save a
/// file, one more question asks whether `goal` rules out downloading. A
/// request with none is left exactly as it was.
pub(crate) fn add_questions(body: &mut Value, space: &ActionSpace, goal: &str) -> Vec<Asked> {
    let Some(questions) = body["questions"].as_object_mut() else {
        return Vec::new();
    };
    let mut asked = Vec::new();
    for (operation, target, action) in shortlisted(space) {
        let target = target.clone();
        let key = key(operation, &target);
        questions.insert(
            key.clone(),
            question::irreversible(operation, &target, action),
        );
        asked.push(Asked {
            key,
            operation: operation.to_string(),
            target,
        });
    }
    let downloads = space
        .targets_for("CLICK")
        .into_iter()
        .flatten()
        .any(|(_, action)| starts_download(action));
    if downloads {
        questions.insert(DOWNLOADS_KEY.into(), question::downloads(goal));
        asked.push(Asked {
            key: DOWNLOADS_KEY.into(),
            operation: "DOWNLOADS".into(),
            target: String::new(),
        });
    }
    asked
}

/// P(yes) from a Noul answer, validated as strictly as a choice: an object
/// whose `type`, when present, is `noul`, with a finite `noul` in [0, 1].
pub(crate) fn noul_probability(answer: &Value) -> Option<f64> {
    let answer = answer.as_object()?;
    if answer.get("type").is_some_and(|kind| kind != "noul") {
        return None;
    }
    answer
        .get("noul")?
        .as_f64()
        .filter(|probability| probability.is_finite() && (0.0..=1.0).contains(probability))
}

/// The answer about the chosen `operation` on `target`: `None` when it was
/// not asked, or its answer is missing or invalid, which the loop treats as
/// irreversible.
pub(crate) fn chosen_probability(
    answers: &Value,
    asked: &[Asked],
    operation: &str,
    target: Option<&str>,
) -> Option<f64> {
    let asked = asked
        .iter()
        .find(|asked| asked.operation == operation && Some(asked.target.as_str()) == target)?;
    noul_probability(&answers[&asked.key])
}

/// The answer whether the goal rules out downloading: `None` when it was
/// not asked, or its answer is missing or invalid, which the loop treats as
/// ruling it out for a click that saves a file.
pub(crate) fn downloads_probability(answers: &Value, asked: &[Asked]) -> Option<f64> {
    asked
        .iter()
        .any(|asked| asked.key == DOWNLOADS_KEY)
        .then(|| noul_probability(&answers[DOWNLOADS_KEY]))
        .flatten()
}

/// What the loop does with the chosen action.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Verdict {
    Dispatch,
    /// Stop `needs_confirmation`, for this reason.
    Confirm(String),
}

/// The gate's answers about the chosen action, as the decision carries them.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Answers {
    /// P(the action cannot be undone).
    pub(crate) irreversible: Option<f64>,
    /// P(the goal rules out downloading).
    pub(crate) downloads_forbidden: Option<f64>,
}

/// Whether the run may act on what the gate stops: authorized, and the
/// decision confident enough that it is the control the goal means.
fn waved_through(authorized: bool, call_confidence: f64) -> bool {
    authorized && call_confidence >= AUTHORIZED_CONFIDENCE
}

/// Why the run did not wave a stopped action through.
fn not_waved(authorized: bool, call_confidence: f64) -> String {
    if authorized {
        format!(
            "and the decision to do it was not confident enough ({call_confidence:.2} < \
             {AUTHORIZED_CONFIDENCE:.2}), so it may be the wrong control"
        )
    } else {
        "and this run is not authorized to".to_string()
    }
}

fn judged(probability: Option<f64>) -> String {
    match probability {
        Some(probability) => format!("P={probability:.2}"),
        None => "the model gave no valid answer about it".to_string(),
    }
}

/// Decide whether the chosen `action`, among the `offered` actions of its
/// observation, may be dispatched:
///
/// - a click the page says saves a file, when the goal rules out
///   downloading (P above [`IRREVERSIBLE_ABOVE`], or no valid answer), may
///   not, authorized or not;
/// - a submit that sets a new password may only in an authorized run whose
///   call confidence is at least [`AUTHORIZED_CONFIDENCE`];
/// - an action the gate asks about may when P(irreversible) is at or below
///   [`IRREVERSIBLE_ABOVE`], and above it only as a new password may. A
///   missing answer counts as irreversible, except for an action whose
///   label names nothing that the question cap left unasked: it is
///   dispatched, as it was before the gate read past the label.
pub(crate) fn verdict(
    action: &Value,
    offered: &[Value],
    answers: Answers,
    authorized: bool,
    call_confidence: f64,
) -> Verdict {
    let label = action["label"].as_str().unwrap_or_default();
    let what = match action["kind"].as_str() {
        Some("enter") => format!("press Enter in {label:?}"),
        _ => format!("click {label:?}"),
    };
    if starts_download(action)
        && answers
            .downloads_forbidden
            .is_none_or(|forbidden| forbidden > IRREVERSIBLE_ABOVE)
    {
        return Verdict::Confirm(format!(
            "Jev did not {what}: it downloads a file, and the goal rules out downloading ({})",
            judged(answers.downloads_forbidden)
        ));
    }
    if changes_credentials(offered, action) {
        if waved_through(authorized, call_confidence) {
            return Verdict::Dispatch;
        }
        return Verdict::Confirm(format!(
            "Jev did not {what}: it would change the account's password or credentials, {}",
            not_waved(authorized, call_confidence)
        ));
    }
    let Some(tier) = tier(action) else {
        return Verdict::Dispatch;
    };
    // Past the question cap, only a label that names a commitment stops:
    // the heading, the address and the form are what the gate reads to ask
    // more, never a reason to stop what it had no room to ask about.
    if answers.irreversible.is_none() && tier != Tier::Named && !had_room(action, offered) {
        return Verdict::Dispatch;
    }
    if answers
        .irreversible
        .is_some_and(|irreversible| irreversible <= IRREVERSIBLE_ABOVE)
        || waved_through(authorized, call_confidence)
    {
        return Verdict::Dispatch;
    }
    Verdict::Confirm(format!(
        "Jev did not {what}: it may make a purchase, payment, send, publish, delete or other \
         change that cannot be undone ({}), {}",
        judged(answers.irreversible),
        not_waved(authorized, call_confidence)
    ))
}

/// The keys of every question the gate may add, to tell them from others.
#[cfg(test)]
pub(crate) fn is_gate_key(key: &str) -> bool {
    key.starts_with("irreversible_click_")
        || key.starts_with("irreversible_enter_")
        || key == DOWNLOADS_KEY
}

#[cfg(test)]
mod tests;
