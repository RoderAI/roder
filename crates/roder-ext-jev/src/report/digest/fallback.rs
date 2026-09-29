//! The digest's account of the fallback: in the header, which driver did
//! what and at what cost, and, when the caller should go on itself, the
//! full browser tools that work on this same tab; among the page content,
//! the fallback's steps and its last word (both read off the page).

use serde_json::Value;

use super::{STEP_CHARS, clip, cut, one_line, plural, seconds, text};

/// The header's first line after a fallback ran: the call's end state, then
/// Jev's own.
pub(super) fn first_line(data: &Value, status_words: &str) -> Option<String> {
    let fallback = &data["fallback"];
    if fallback["ran"] != Value::Bool(true) {
        return None;
    }
    Some(format!(
        "Jev: {status_words}, after a fallback. Jev itself stopped {} ({}); the model {} went \
         on in the same tab with Roder's full browser tools.",
        super::super::status_words(text(&data["jev_status"])),
        text(&fallback["trigger"]["why"]),
        text(&fallback["model"]),
    ))
}

/// Who did what, at what cost: Jev's time is never the fallback's.
pub(super) fn drivers(data: &Value) -> Option<String> {
    let drivers = data["drivers"].as_array()?;
    let fallback = drivers
        .iter()
        .find(|driver| driver["driver"] == "fallback")?;
    let jev = drivers.iter().find(|driver| driver["driver"] == "jev")?;
    let usage = &fallback["usage"];
    let tokens = match usage["incomplete"] == Value::Bool(true) {
        true => "tokens not all reported".to_string(),
        false => format!(
            "{} input and {} output tokens",
            usage["input_tokens"].as_u64().unwrap_or(0),
            usage["output_tokens"].as_u64().unwrap_or(0)
        ),
    };
    Some(format!(
        "Drivers: Jev {}, {}, {}; fallback ({}) {}, {}, {}, {tokens}.",
        plural(jev["actions"].as_u64().unwrap_or(0), "action", "actions"),
        plural(
            jev["decisions"].as_u64().unwrap_or(0),
            "decision",
            "decisions"
        ),
        seconds(jev["elapsed_ms"].as_u64().unwrap_or(0)),
        text(&fallback["model"]),
        plural(
            fallback["actions"].as_u64().unwrap_or(0),
            "tool call",
            "tool calls"
        ),
        plural(
            fallback["model_calls"].as_u64().unwrap_or(0),
            "model call",
            "model calls"
        ),
        seconds(fallback["elapsed_ms"].as_u64().unwrap_or(0)),
    ))
}

/// The hand-over: when the caller should go on with the full tools on this
/// tab, what they are, where, and what not to do instead.
pub(super) fn handover(data: &Value) -> Option<String> {
    let fallback = &data["fallback"];
    let tools = fallback["tools"]
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    if tools.is_empty() {
        return None;
    }
    let tab = fallback["tab"]
        .as_str()
        .map_or("this thread's Jev tab".to_string(), |tab| {
            format!("this thread's Jev tab, {tab}")
        });
    let who = match fallback["ran"] == Value::Bool(true) {
        true => "the fallback stopped too",
        false => "Jev could not progress",
    };
    let mut next = format!(
        "Next: {who} ({}). Roder's full browser tools work on this same tab ({tab}), with its \
         page and state as left: {}. Go on with them from where it stopped, starting with \
         jev_tab_look (or jev_tab_screenshot), instead of calling jev_browse again with the \
         same goal or giving up. They act only in this tab; stop before signing in, \
         reserving, paying or sending personal details unless the user asked for that step, \
         and never get around a site's block.",
        text(&fallback["trigger"]["why"]),
        tools.join(", ")
    );
    if let Some(why) = fallback["not_run_because"].as_str() {
        next.push_str(&format!(
            " (The automatic fallback did not run: {}.)",
            cut(&one_line(why), 160)
        ));
    }
    Some(next)
}

/// The fallback's steps and last word, page-derived.
pub(super) fn steps(data: &Value) -> Vec<String> {
    let fallback = &data["fallback"];
    let actions = fallback["actions"].as_array().cloned().unwrap_or_default();
    let mut out = Vec::new();
    if !actions.is_empty() {
        out.push("What the fallback did:".to_string());
        let lines = actions
            .iter()
            .map(|action| {
                let mut line = format!(" {}. {}", action["step"], text(&action["tool"]));
                if let Some(target) = action["target"].as_str() {
                    line.push_str(&format!(" {}", cut(&one_line(target), 60)));
                }
                line.push_str(&format!(" → {}", one_line(text(&action["result"]))));
                clip(&line, STEP_CHARS)
            })
            .collect::<Vec<_>>();
        match lines.len() {
            0..=10 => out.extend(lines),
            n => {
                out.extend(lines[..3].iter().cloned());
                out.push(format!("  … ({} more steps)", n - 9));
                out.extend(lines[n - 6..].iter().cloned());
            }
        }
    }
    if let Some(message) = fallback["message"]
        .as_str()
        .filter(|message| !message.trim().is_empty())
    {
        out.push(format!(
            "The fallback's last word: {}",
            cut(&one_line(message), 300)
        ));
    }
    out
}
