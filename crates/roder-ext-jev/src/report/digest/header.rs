//! The digest's header and session summary: status, the tab, today's date,
//! the page's address and title, the outcome and what to do next; then the
//! session's tabs, totals and earlier calls.

use serde_json::Value;

use super::super::status_words;
use super::{EARLIER_CALLS, cut, one_line, plural, seconds, text};

pub(super) fn header(data: &Value, status: &str, now: &str) -> Vec<String> {
    let session = &data["session"];
    let mut first = format!("Jev: {}.", status_words(status));
    if let Some(call) = session["call"].as_u64() {
        first.push_str(&format!(
            " Call {call} in this thread's Jev browser session; {}.",
            tab_clause(session, call)
        ));
    }
    let mut lines = vec![first, today(session, now)];
    let url = text(&data["url"]);
    if !url.is_empty() {
        lines.push(format!(
            "Now at (page-supplied): {}",
            cut(&one_line(url), 300)
        ));
    }
    let title = one_line(text(&data["title"]));
    let http = data["page"]["http_status"]
        .as_u64()
        .map(|code| format!("HTTP {code}"));
    match (title.is_empty(), http) {
        (false, Some(http)) => lines.push(format!(
            "Title (page-supplied): {}   {http}",
            cut(&title, 150)
        )),
        (false, None) => lines.push(format!("Title (page-supplied): {}", cut(&title, 150))),
        (true, Some(http)) => lines.push(format!("Page status: {http}")),
        (true, None) => {}
    }
    lines.push(outcome(data, status));
    if let Some(next) = next(data, status) {
        lines.push(next);
    }
    lines
}

/// Today's date, and what "tonight" means: the date the session began on,
/// which is today's unless a flow ran past midnight.
fn today(session: &Value, now: &str) -> String {
    match session["began_on"].as_str() {
        Some(began) if !began.is_empty() && !now.starts_with(began) => format!(
            "Today: {now}. The date has changed since this session began on {began}; a \
             \"tonight\" or \"today\" the user asked for then means {began}."
        ),
        _ => format!("Today: {now}. \"Tonight\" means this date."),
    }
}

/// Which tab the call used and how it came to be on it.
fn tab_clause(session: &Value, call: u64) -> String {
    let Some(tab) = session["tab"].as_str() else {
        return "no tab open".into();
    };
    match session["tab_note"].as_str() {
        Some("continued") if call > 1 => format!(
            "same tab as before ({tab}, continued from where call {} ended)",
            call - 1
        ),
        Some("navigated") => match session["tab_detail"].as_str() {
            Some(detail) => format!("same tab as before ({tab}), loaded the url given: {detail}"),
            None => format!("same tab as before ({tab}), loaded the url given"),
        },
        Some("reopened") => format!(
            "tab {tab} (reopened: {})",
            session["tab_detail"]
                .as_str()
                .unwrap_or("the earlier tab was closed")
        ),
        _ if call == 1 => format!("tab {tab}, opened for this session's first call"),
        _ => format!("tab {tab}, a new tab"),
    }
}

fn outcome(data: &Value, status: &str) -> String {
    let actions = data["actions"].as_array().map_or(0, Vec::len) as u64;
    let decisions = data["model_calls"].as_u64().unwrap_or(0);
    let spent = format!(
        "{}, {}, {}",
        plural(actions, "action", "actions"),
        plural(decisions, "decision", "decisions"),
        seconds(data["elapsed_ms"].as_u64().unwrap_or(0))
    );
    match status {
        "done" => format!("Result: Jev judged the goal met ({spent})."),
        "blocked" => format!("Result: Jev could not make progress ({spent})."),
        "access_denied" => format!("Result: the site refused automated access ({spent})."),
        "budget_exceeded" => format!("Result: Jev ran out of its action budget ({spent})."),
        "timed_out" => format!("Result: the call ran out of time ({spent})."),
        "needs_input" => format!("Result: a field needs a value the goal does not give ({spent})."),
        "needs_confirmation" => {
            format!("Result: Jev stopped before an action that may not be undone ({spent}).")
        }
        "unavailable" => format!("Result: a model provider could not be reached ({spent})."),
        _ => format!("Result: the call failed ({spent})."),
    }
}

fn next(data: &Value, status: &str) -> Option<String> {
    let has_tab = data["session"]["tab"].is_string();
    let mut next = match status {
        "done" => {
            let mut next = String::from(
                "Next: Check the page below against the goal; Jev's done is a judgement, not a \
                 proof.",
            );
            if has_tab {
                next.push_str(
                    " To keep going from this page, call jev_browse again with url \"\" and tab \
                     \"current\".",
                );
            }
            next.push_str(
                " Do not sign in, reserve, pay or submit personal details unless the user asked \
                 for that exact step.",
            );
            next
        }
        _ => format!("Next: {}", data["next_step"].as_str()?),
    };
    let blank_page = data["observed_elements"].as_u64() == Some(0)
        && !text(&data["visible_text"]).trim().is_empty();
    if status == "blocked" && blank_page {
        next.push_str(
            " Jev targets links, buttons, form fields, ARIA-role elements and elements with a \
             click listener, a tabindex or a pointer cursor, and this page exposed none: canvas \
             drawings, drags and controls reached only by hover are invisible to it. Try \
             another real page or a different browser tool, and report this rather than \
             substituting a locally built page.",
        );
    } else if status != "done"
        && data["text_calls"].as_u64() == Some(0)
        && data["text_model"].is_null()
    {
        next.push_str(
            " No text model is configured, so Jev cannot type: sign in with `roder auth login \
             codex`, configure a Roder chat-completions provider or set JEV_TEXT_MODEL_API_KEY.",
        );
    }
    Some(next)
}

pub(super) fn session(data: &Value) -> Vec<String> {
    let session = &data["session"];
    let Some(call) = session["call"].as_u64() else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    let totals = &session["totals"];
    let open = session["tabs"].as_array().map_or(0, Vec::len);
    let current = session["tab"]
        .as_str()
        .map(|tab| format!(" ({tab} current)"))
        .unwrap_or_default();
    lines.push(format!(
        "Session: tabs open {open} of {}{current}. This session so far: {}, {}, {}, {}.",
        session["max_tabs"].as_u64().unwrap_or(3),
        plural(totals["calls"].as_u64().unwrap_or(call), "call", "calls"),
        plural(totals["actions"].as_u64().unwrap_or(0), "action", "actions"),
        plural(
            totals["decisions"].as_u64().unwrap_or(0),
            "decision",
            "decisions"
        ),
        plural(
            totals["text_calls"].as_u64().unwrap_or(0),
            "value typed",
            "values typed"
        ),
    ));
    if let Some(moved) = session["moved_to"].as_str() {
        lines.push(format!(
            "The tab was at {} when this call began (changed since the last call); Jev went on \
             from there.",
            cut(&one_line(moved), 200)
        ));
    }
    if let Some(waited) = session["waited_ms"].as_u64() {
        lines.push(format!(
            "Waited {} for the previous jev_browse call on this tab.",
            seconds(waited)
        ));
    }
    let earlier = session["earlier_calls"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if !earlier.is_empty() {
        lines.push("Earlier calls in this session:".into());
        for record in earlier.iter().rev().take(EARLIER_CALLS).rev() {
            lines.push(format!(
                " {}. \"{}\" → {} at {} ({})",
                record["n"],
                cut(&one_line(text(&record["goal"])), 70),
                status_words(record["status"].as_str().unwrap_or("unknown")),
                cut(&one_line(text(&record["end_url"])), 80),
                plural(record["actions"].as_u64().unwrap_or(0), "action", "actions"),
            ));
        }
    }
    lines
}
