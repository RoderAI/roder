//! The text the caller reads: what the run did, why it stopped, the page it
//! ended on, what to do next, and where the thread's Jev session stands.
//! Roder gives the model only a tool result's text, never its data, so the
//! page itself has to be in it (see [`digest`]).

#[cfg(test)]
mod cover_tests;
mod digest;
#[cfg(test)]
mod hint_tests;
#[cfg(test)]
mod not_clicked_tests;
#[cfg(test)]
mod omission_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod toggle_tests;

use chrono::{DateTime, Local};
pub(crate) use digest::digest;

/// Today's date as the caller should read it, with the local time zone:
/// "Mon 2026-09-28 (America/Los_Angeles, UTC-07:00)".
pub(crate) fn today() -> String {
    today_at(Local::now(), iana_time_zone::get_timezone().ok())
}

fn today_at(now: DateTime<Local>, zone: Option<String>) -> String {
    format!("{} ({})", now.format("%a %Y-%m-%d"), zone_of(now, zone))
}

/// Today's date alone, as [`now`] begins: "Mon 2026-09-28".
pub(crate) fn date() -> String {
    Local::now().format(DATE).to_string()
}

const DATE: &str = "%a %Y-%m-%d";

/// Today with the time of day, for a result: "Mon 2026-09-28, 17:42 local
/// time (America/Los_Angeles, UTC-07:00)".
pub(crate) fn now() -> String {
    now_at(Local::now(), iana_time_zone::get_timezone().ok())
}

fn now_at(now: DateTime<Local>, zone: Option<String>) -> String {
    format!(
        "{} local time ({})",
        now.format("%a %Y-%m-%d, %H:%M"),
        zone_of(now, zone)
    )
}

fn zone_of(now: DateTime<Local>, zone: Option<String>) -> String {
    let offset = now.format("UTC%:z");
    match zone {
        Some(zone) => format!("{zone}, {offset}"),
        None => offset.to_string(),
    }
}

/// The text `jev_browse` returns for `data`, which first gains the
/// `next_step` hint for its status.
pub(crate) fn tool_text(data: &mut serde_json::Value) -> String {
    if let Some(hint) = data["status"].as_str().and_then(next_step) {
        data["next_step"] = serde_json::json!(hint);
    }
    digest(data, &now())
}

/// What the caller should do after a run that did not finish, one fixed
/// sentence per status. The wording started from fastbrowse's MCP server
/// (MIT).
pub(crate) fn next_step(status: &str) -> Option<&'static str> {
    Some(match status {
        "blocked" => {
            "Read the page below to see where Jev stopped. Then call again with url \"\" and a \
             narrower goal that names the control to use, start from a more specific page, or \
             use another browser tool family if one is available (chrome_*, browser_use_*, \
             webwright.*)."
        }
        "budget_exceeded" => {
            "Split the task into smaller goals, and run each with url \"\" from the page where \
             it starts."
        }
        "timed_out" => {
            "Retry with a larger timeout_seconds, or split the task and go on with url \"\" \
             from the page reached."
        }
        "needs_input" => {
            "Why Jev stopped, below, names the field Jev had no value for. Do that step \
             yourself, or call again with url \"\" and a goal that gives the value; whatever \
             the goal holds is sent to the hosted decision service and the typing model and \
             stored in the transcript. A text model must be configured."
        }
        "unavailable" => "A model provider could not be reached; wait a moment and retry.",
        "needs_confirmation" => {
            "Jev stopped before an action that may not be undone, named in why Jev stopped \
             below. Ask the user to confirm that exact step; only then call again with url \"\", \
             authorize_irreversible: true and a goal that asks for that step."
        }
        "access_denied" => {
            "This site refused Jev's automated access. Do not retry it or try to get around \
             the block. If the user asked for this particular site, tell them it refused \
             automated access and ask how to go on. Otherwise the task is not finished: if \
             another site offers the same thing, go on there now by calling again with its url \
             and tab \"current\" (it loads in this same tab); moving to a site the user did \
             not name needs no confirmation from the user. Tell the user only when no other \
             site will do."
        }
        "error" => "Read why Jev stopped, below; retry only once its cause is fixed.",
        _ => return None,
    })
}

/// How the text names each status.
pub(crate) fn status_words(status: &str) -> &str {
    match status {
        "budget_exceeded" => "ran out of budget",
        "timed_out" => "timed out",
        "needs_input" => "needs input",
        "unavailable" => "could not reach its model",
        "needs_confirmation" => "needs confirmation",
        "access_denied" => "access denied",
        "error" => "failed",
        other => other,
    }
}
