//! Rendering of recall results into the readable "retrieved context" string the
//! answerer (or the model) consumes. Bi-temporal status, validity windows, and
//! supersession reasons are surfaced so as-of / audit-replay / contradiction
//! questions are answerable from the context alone.

use time::OffsetDateTime;

use crate::model::{FactStatus, TemporalFact};
use crate::store::RecallResult;

fn date_short(dt: OffsetDateTime) -> String {
    dt.date().to_string()
}

fn fact_line(fact: &TemporalFact, now: OffsetDateTime) -> String {
    let status = fact.status(now);
    let mut window = format!("valid from {}", date_short(fact.valid_at));
    if let Some(invalid) = fact.invalid_at {
        window.push_str(&format!(" until {}", date_short(invalid)));
    }
    let mut line = format!("- [{}] {} ({window}", status.as_str(), fact.text.trim());
    if status == FactStatus::Retracted
        && let Some(expired) = fact.expired_at {
            line.push_str(&format!("; record retracted {}", date_short(expired)));
        }
    if let Some(reason) = &fact.supersession_reason {
        line.push_str(&format!("; supersedes prior because: {reason}"));
    }
    line.push(')');
    if !fact.provenance.is_empty() {
        line.push_str(&format!("  [sources: {}]", fact.provenance.join(", ")));
    }
    line
}

/// Render a recall result as a context block. When the query was an `as_of`
/// snapshot, the header states the as-of instant and facts that have *since*
/// changed are flagged (audit replay).
pub fn render_recall(result: &RecallResult) -> String {
    if result.hits.is_empty() {
        return match result.as_of.is_current() {
            true => "No matching facts found.".to_string(),
            false => format!(
                "No facts were on record as of {}.",
                date_short(result.as_of.anchor(result.now))
            ),
        };
    }

    let mut out = String::new();
    if result.as_of.is_current() {
        out.push_str(&format!("{} relevant fact(s):", result.hits.len()));
    } else {
        out.push_str(&format!(
            "{} fact(s) the organization believed as of {} (current status noted):",
            result.hits.len(),
            date_short(result.as_of.anchor(result.now)),
        ));
    }
    for hit in &result.hits {
        out.push('\n');
        out.push_str(&fact_line(&hit.fact, result.now));
        // Audit replay: if viewing the past, flag what has since changed.
        if !result.as_of.is_current() {
            match hit.fact.status(result.now) {
                FactStatus::Current => {}
                FactStatus::Superseded => out.push_str("  ⚠ has since been superseded"),
                FactStatus::Invalidated => out.push_str("  ⚠ no longer true today"),
                FactStatus::Retracted => out.push_str("  ⚠ record since retracted"),
            }
        }
    }

    if !result.contradictions.is_empty() {
        out.push_str("\nUnresolved contradictions detected:");
        for pair in &result.contradictions {
            out.push_str(&format!(
                "\n  • \"{}\" conflicts with \"{}\" (same subject, overlapping validity)",
                pair.a.text.trim(),
                pair.b.text.trim(),
            ));
        }
    }
    out
}
