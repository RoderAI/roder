use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use super::ValidatedQuoteSpan;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DreamLinkKind {
    Invalidates,
    Supersedes,
    Contradicts,
    Refines,
}

impl DreamLinkKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Invalidates => "invalidates",
            Self::Supersedes => "supersedes",
            Self::Contradicts => "contradicts",
            Self::Refines => "refines",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DerivedEventCandidate {
    pub id: String,
    pub entity_ids: Vec<String>,
    pub predicate_family: String,
    pub statement_text: String,
    pub quote: ValidatedQuoteSpan,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub valid_at: Option<OffsetDateTime>,
    #[serde(default)]
    pub supersedes_event_ids: Vec<String>,
    #[serde(default)]
    pub correction_of_event_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DreamLinkCandidate {
    pub id: String,
    pub source_event_id: String,
    pub target_event_id: String,
    pub kind: DreamLinkKind,
    pub evidence_quote: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvalidationDecision {
    Candidate(DreamLinkCandidate),
    Rejected(String),
}

pub fn propose_invalidation_links(
    existing: &[DerivedEventCandidate],
    incoming: &[DerivedEventCandidate],
) -> Vec<InvalidationDecision> {
    let mut decisions = Vec::new();
    for new_event in incoming {
        for old_event in existing {
            if !related(old_event, new_event) {
                continue;
            }
            if chronologically_impossible(old_event, new_event) {
                decisions.push(InvalidationDecision::Rejected(format!(
                    "{} cannot invalidate newer event {}",
                    new_event.id, old_event.id
                )));
                continue;
            }

            if new_event
                .supersedes_event_ids
                .iter()
                .any(|id| id == &old_event.id)
            {
                decisions.push(InvalidationDecision::Candidate(link(
                    new_event,
                    old_event,
                    DreamLinkKind::Supersedes,
                    "fact supersedes metadata",
                )));
                continue;
            }
            if new_event
                .correction_of_event_ids
                .iter()
                .any(|id| id == &old_event.id)
            {
                decisions.push(InvalidationDecision::Candidate(link(
                    new_event,
                    old_event,
                    DreamLinkKind::Invalidates,
                    "fact correction metadata",
                )));
                continue;
            }

            if explicit_replacement(&new_event.statement_text) {
                decisions.push(InvalidationDecision::Candidate(link(
                    new_event,
                    old_event,
                    DreamLinkKind::Invalidates,
                    "explicit replacement language",
                )));
            } else if explicit_refinement(&new_event.statement_text) {
                decisions.push(InvalidationDecision::Candidate(link(
                    new_event,
                    old_event,
                    DreamLinkKind::Refines,
                    "explicit refinement language",
                )));
            } else if looks_like_unresolved_conflict(old_event, new_event) {
                decisions.push(InvalidationDecision::Rejected(format!(
                    "{} and {} may conflict, but no explicit resolution was found",
                    new_event.id, old_event.id
                )));
            }
        }
    }

    decisions.sort_by(|a, b| decision_key(a).cmp(&decision_key(b)));
    decisions.dedup_by(|a, b| decision_key(a) == decision_key(b));
    decisions
}

fn related(left: &DerivedEventCandidate, right: &DerivedEventCandidate) -> bool {
    left.predicate_family == right.predicate_family
        && left
            .entity_ids
            .iter()
            .any(|left_id| right.entity_ids.iter().any(|right_id| right_id == left_id))
}

fn chronologically_impossible(old: &DerivedEventCandidate, new: &DerivedEventCandidate) -> bool {
    matches!((old.valid_at, new.valid_at), (Some(old_at), Some(new_at)) if new_at < old_at)
}

fn explicit_replacement(text: &str) -> bool {
    let text = text.to_lowercase();
    [
        "replaces",
        "replacement for",
        "supersedes",
        "no longer applies",
        "is no longer",
        "instead of",
        "corrects",
        "correction:",
        "updated policy",
        "new policy",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

fn explicit_refinement(text: &str) -> bool {
    let text = text.to_lowercase();
    [
        "clarifies",
        "refines",
        "more specifically",
        "valid from",
        "effective from",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

fn looks_like_unresolved_conflict(
    old: &DerivedEventCandidate,
    new: &DerivedEventCandidate,
) -> bool {
    old.predicate_family == new.predicate_family && old.statement_text != new.statement_text
}

fn link(
    source: &DerivedEventCandidate,
    target: &DerivedEventCandidate,
    kind: DreamLinkKind,
    reason: &str,
) -> DreamLinkCandidate {
    DreamLinkCandidate {
        id: format!("dream_link:{}:{}:{}", kind.as_str(), source.id, target.id),
        source_event_id: source.id.clone(),
        target_event_id: target.id.clone(),
        kind,
        evidence_quote: source.quote.text.clone(),
        reason: reason.to_string(),
    }
}

fn decision_key(decision: &InvalidationDecision) -> String {
    match decision {
        InvalidationDecision::Candidate(candidate) => format!("candidate:{}", candidate.id),
        InvalidationDecision::Rejected(reason) => format!("rejected:{reason}"),
    }
}
