//! The page-supplied sections of the digest: why Jev stopped, what it did,
//! the frames it read, headings and the page text.

use serde_json::Value;

use super::{
    FRAME_CHARS, FRAMES, HEADING_CHARS, JOINED_LINE, NOT_CLICKED_CHARS, REASON_CHARS, SHORT_LINE,
    STEP_CHARS, STEP_LINES, TEXT_LINE_CHARS, clip, cut, keep, one_line, text,
};
use crate::agent::{REPEAT_CONFIDENCE, tidy_cover};

pub(super) fn reason(data: &Value) -> Vec<String> {
    let who = match data["fallback"]["ran"] == Value::Bool(true) {
        true => "the fallback",
        false => "Jev",
    };
    match data["stopped_because"].as_str() {
        Some(reason) if !reason.trim().is_empty() => {
            vec![format!(
                "Why {who} stopped: {}",
                cut(&one_line(reason), REASON_CHARS)
            )]
        }
        _ => Vec::new(),
    }
}

pub(super) fn steps(data: &Value) -> Vec<String> {
    let actions = data["actions"].as_array().cloned().unwrap_or_default();
    if actions.is_empty() {
        return Vec::new();
    }
    let lines = actions.iter().map(step).collect::<Vec<_>>();
    let mut out = vec!["What Jev did:".to_string()];
    if lines.len() < STEP_LINES {
        out.extend(lines);
    } else {
        let (head, tail) = (3, STEP_LINES - 4);
        out.extend(lines[..head].iter().cloned());
        out.push(format!("  … ({} more steps)", lines.len() - head - tail));
        out.extend(lines[lines.len() - tail..].iter().cloned());
    }
    out
}

/// The click a `done` run ended on instead of making: what it was, where it
/// sat, and the click before it that it was read as repeating. Its labels
/// are page text, so it sits among the page-supplied lines.
pub(super) fn not_clicked(data: &Value) -> Vec<String> {
    let held = &data["suppressed_click"];
    let Some(label) = held["label"].as_str() else {
        return Vec::new();
    };
    let named = |label: &str, context: &Value| {
        let mut named = format!("\"{}\"", cut(&one_line(label), 60));
        if let Some(context) = context
            .as_str()
            .filter(|context| !context.trim().is_empty())
        {
            named.push_str(&format!(" (in: {})", cut(&one_line(context), 50)));
        }
        named
    };
    let confidence = held["confidence"]
        .as_f64()
        .map(|confidence| format!(" at confidence {confidence:.2}"))
        .unwrap_or_default();
    let line = match held["kind"].as_str() {
        Some("twin_control") => format!(
            "Not clicked: {}, chosen{confidence} right after a click on {} that changed the page. \
             Below {REPEAT_CONFIDENCE:.2} confidence Jev reads that as the goal being met.",
            named(label, &held["context"]),
            named(label, &held["previous_context"]),
        ),
        _ => format!(
            "Not clicked: {} again, chosen{confidence} right after that click changed the page. \
             Below {REPEAT_CONFIDENCE:.2} confidence Jev reads that as the goal being met.",
            named(label, &held["context"]),
        ),
    };
    vec![clip(&line, NOT_CLICKED_CHARS)]
}

fn step(action: &Value) -> String {
    let mut line = format!(
        " {}. {} \"{}\"",
        action["step"],
        text(&action["kind"]),
        cut(&one_line(text(&action["action"])), 70)
    );
    if let Some(context) = action["context"].as_str() {
        line.push_str(&format!(" (in: {})", cut(&one_line(context), 50)));
    }
    if let Some(typed) = action["text"].as_str() {
        line.push_str(&format!(" with \"{}\"", cut(&one_line(typed), 40)));
    }
    let mut outcome = Vec::new();
    if let Some(how) = action["uncovered"].as_str() {
        outcome.push(format!("it was covered; Jev {}", one_line(how)));
    }
    if action["covered"] == Value::Bool(true) {
        outcome.push(match covered_by(action) {
            Some(cover) => format!("covered by \"{cover}\"; nothing was done"),
            None => "covered by another element; nothing was done".to_string(),
        });
    }
    if let Some(refused) = action["refused"].as_str() {
        outcome.push(format!("the page refused it: {}", one_line(refused)));
    }
    match action["page_changed"].as_bool() {
        Some(true) => outcome.push("page changed".into()),
        Some(false) if action["covered"] != Value::Bool(true) => {
            outcome.push("no visible change".into())
        }
        _ => {}
    }
    if action["opened_tab"] == Value::Bool(true) {
        outcome.push("opened a new tab".into());
    }
    if let Some(effect) = action["effect"]
        .as_str()
        .filter(|effect| !effect.is_empty())
    {
        outcome.push(one_line(effect));
    }
    for dialog in action["dialogs"].as_array().into_iter().flatten() {
        outcome.push(format!(
            "{} dialog \"{}\" {}",
            text(&dialog["type"]),
            cut(&one_line(text(&dialog["message"])), 60),
            if dialog["accepted"] == Value::Bool(true) {
                "accepted"
            } else {
                "dismissed"
            }
        ));
    }
    if !outcome.is_empty() {
        line.push_str(" → ");
        line.push_str(&outcome.join("; "));
    }
    clip(&line, STEP_CHARS)
}

/// What covered a step's target, as the page names it. Page text, so it
/// is held to the same line and length the record keeps it to, whoever
/// built the data.
fn covered_by(action: &Value) -> Option<String> {
    tidy_cover(&one_line(action["covered_by"].as_str()?))
}

pub(super) fn frames(data: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    for frame in data["page"]["frames"]
        .as_array()
        .into_iter()
        .flatten()
        .take(FRAMES)
    {
        lines.push(format!(
            "Frame {} (read-only: Jev sees it but cannot act in it):",
            cut(&one_line(text(&frame["origin"])), 80)
        ));
        lines.push(format!(
            "  {}",
            clip(&one_line(text(&frame["text"])), FRAME_CHARS)
        ));
    }
    lines
}

pub(super) fn headings(data: &Value) -> Vec<String> {
    let headings = data["page"]["headings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(one_line)
        .filter(|heading| !heading.is_empty())
        .collect::<Vec<_>>();
    if headings.is_empty() {
        return Vec::new();
    }
    vec![format!(
        "Headings: {}",
        cut(&headings.join(" · "), HEADING_CHARS)
    )]
}

/// The page text: blank lines and repeats dropped, runs of short lines
/// joined, frame lines left to their own section.
pub(super) fn page_text(data: &Value, chars: usize, lines: usize) -> Vec<String> {
    let raw = text(&data["visible_text"]);
    let mut seen = std::collections::HashSet::new();
    let mut joined: Vec<String> = Vec::new();
    let mut run = String::new();
    for line in raw.lines() {
        let line = one_line(line);
        if line.is_empty() || line.starts_with("[frame ") || !seen.insert(line.clone()) {
            continue;
        }
        let short = line.chars().count() < SHORT_LINE;
        if short && run.chars().count() + line.chars().count() + 3 <= JOINED_LINE {
            if !run.is_empty() {
                run.push_str(" · ");
            }
            run.push_str(&line);
            continue;
        }
        if !run.is_empty() {
            joined.push(std::mem::take(&mut run));
        }
        match short {
            true => run = line,
            false => joined.push(cut(&line, TEXT_LINE_CHARS)),
        }
    }
    if !run.is_empty() {
        joined.push(run);
    }
    if joined.is_empty() || lines < 3 || chars < 60 {
        return Vec::new();
    }
    let mut out = vec!["Text:".to_string()];
    out.extend(joined.into_iter().map(|line| format!("  {line}")));
    keep(out, chars, lines)
}
