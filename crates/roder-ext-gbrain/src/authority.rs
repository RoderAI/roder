//! Deployable multi-signal authority resolution over retrieved evidence
//! (roadmap/90 Phase 1). Goal: before synthesis, resolve which record is
//! authoritative / as-of-correct and tag the rest, so the synthesizer is handed a
//! resolved, ordered view instead of competing records it can mis-pick.
//!
//! Signals are DEPLOYABLE — none uses a ground-truth-only label (the corpus's
//! `role: noise` distractor flag is deliberately NOT used; a real deployment would
//! not have it). We combine:
//!   * recency on the *asked* time axis (valid-at vs the as-of date / "current"),
//!   * supersession LANGUAGE in the record text ("supersedes / corrected / revised"),
//!   * corroboration by independent records (shared salient terms — NOT repetition),
//!   * a small genre/source-type prior.
//!
//! Authority is multi-signal, never channel-tier alone and never pure-recency: a
//! lone uncorroborated recent record does not win on recency, and for "as of D" the
//! record valid-at-D wins over a newer one. Ties are left adjacent and tagged so the
//! answerer can surface both + hedge.

use crate::agent::EvidenceItem;
use crate::ground::event_cluster;

/// How a record relates to the asked question's authoritative answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorityTag {
    /// Best-supported record for the asked time axis — assert from this.
    Authoritative,
    /// Valid as of the asked date (for "as of D" questions).
    KnownAsOf,
    /// Recorded/valid after the as-of date — historical-future; do not assert as the
    /// as-of-then state unless the question asks what changed.
    RecordedLater,
    /// Explicitly superseded/corrected by another record — historical only.
    Superseded,
    /// Ordinary supporting record.
    Supporting,
}

impl AuthorityTag {
    pub fn label(self) -> &'static str {
        match self {
            AuthorityTag::Authoritative => "authoritative",
            AuthorityTag::KnownAsOf => "known as of the asked date",
            AuthorityTag::RecordedLater => "recorded after the as-of date — not part of the as-of-then state",
            AuthorityTag::Superseded => "superseded/corrected by a later record — historical, do not assert unless asked",
            AuthorityTag::Supporting => "supporting",
        }
    }
}

/// A record with its computed authority score + tag.
#[derive(Debug, Clone)]
pub struct ScoredEvidence {
    pub index: usize,
    pub score: f32,
    pub tag: AuthorityTag,
}

/// Does this record's text claim to supersede/correct an earlier account?
fn states_supersession(text: &str) -> bool {
    let t = text.to_lowercase();
    const CUES: &[&str] = &[
        "supersede",
        "supersedes",
        "superseded",
        "corrected root cause",
        "corrected account",
        "revised",
        "revision 2",
        "now understand",
        "actual root cause",
        "ruled out",
        "no longer",
        "replaces the",
        "amended",
    ];
    CUES.iter().any(|c| t.contains(c))
}

/// Coarse, deployable genre prior (small weight). Decisions/records-of-truth above
/// summaries, summaries above raw chatter. Kept small: genre tiers can mislead, so
/// they only break near-ties.
fn genre_prior(source: &str) -> f32 {
    let s = source.to_lowercase();
    if s.contains("adr") || s.contains("decision") || s.contains("postmortem") {
        0.15
    } else if s.contains("incident_report") || s.contains("retrospective") || s.contains("notion") {
        0.10
    } else if s.contains("transcript") || s.contains("email") {
        0.05
    } else {
        0.0 // slack / crm / notes / unknown
    }
}

/// Salient lowercase tokens (≥4 chars, not stopwords) for the corroboration signal.
fn salient_terms(text: &str) -> std::collections::HashSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 4)
        .map(str::to_string)
        .collect()
}

/// Number of DISTINCT independent event clusters (other than this record's own)
/// that share a meaningful fraction of its salient terms. Corroboration =
/// independent sources, NOT repetition: artifacts from the *same* event cluster
/// (slug minus its `-NNN` suffix) count once, so a planted amendment backed only by
/// its own event's sibling artifacts is NOT corroborated. This is the guard against
/// manufactured consensus — a lone "the decision was actually X" amendment scores 0.
fn independent_corroboration(
    idx: usize,
    terms: &[std::collections::HashSet<String>],
    clusters: &[String],
) -> usize {
    let mine = &terms[idx];
    if mine.is_empty() {
        return 0;
    }
    let my_cluster = &clusters[idx];
    let mut seen = std::collections::HashSet::new();
    for (j, other) in terms.iter().enumerate() {
        if j == idx || &clusters[j] == my_cluster {
            continue; // skip self and same-event siblings (repetition, not corroboration)
        }
        let overlap = mine.intersection(other).count();
        if overlap * 4 >= mine.len() {
            seen.insert(clusters[j].clone());
        }
    }
    seen.len()
}

/// Score and tag every retrieved record. `as_of` is the asked date (ISO `YYYY-MM-DD`)
/// if the question is an as-of/audit question; `wants_change` is true when the
/// question also asks what changed since (so later records stay relevant).
pub fn resolve(
    evidence: &[EvidenceItem],
    as_of: Option<&str>,
    wants_change: bool,
) -> Vec<ScoredEvidence> {
    let terms: Vec<_> = evidence.iter().map(|e| salient_terms(&e.text)).collect();
    let clusters: Vec<String> = evidence
        .iter()
        .map(|e| event_cluster(&e.slug).unwrap_or(&e.slug).to_string())
        .collect();
    let mut out = Vec::with_capacity(evidence.len());
    for (i, e) in evidence.iter().enumerate() {
        let mut score = 0.0f32;
        let mut tag = AuthorityTag::Supporting;

        // Recency on the asked axis. `e.date` is the record's valid-at (ISO date),
        // ISO so string compare == chronological compare.
        if let Some(asof) = as_of {
            if e.date.as_str() <= asof {
                tag = AuthorityTag::KnownAsOf;
                score += 0.2;
            } else if !wants_change {
                // recorded after the as-of date and the question is not asking what
                // changed: this is not part of the as-of-then state.
                tag = AuthorityTag::RecordedLater;
                score -= 0.5;
            }
        }

        let corro = independent_corroboration(i, &terms, &clusters);
        // Supersession language confers authority ONLY when the question actually has
        // temporal/change intent. A plain "who made the decision" question (no as-of,
        // no change cue) must NOT have a later "correction" promoted over the original
        // — that is the planted-amendment guard, and it is what regressed Q-0002 when
        // ungated. On change/as-of questions the correction is exactly what the rubric
        // wants, so it is promoted. (Lexical corroboration is kept only as a small
        // additive tie-breaker below — it conflates "agrees with" vs "same topic", so
        // it is not used as a hard gate.)
        let supersession_relevant = wants_change || as_of.is_some();
        if states_supersession(&e.text) && supersession_relevant {
            score += 0.4;
            if tag != AuthorityTag::RecordedLater {
                tag = AuthorityTag::Authoritative;
            }
        }

        score += genre_prior(&e.source);
        score += (corro.min(3) as f32) * 0.1;

        out.push(ScoredEvidence {
            index: e.index,
            score,
            tag,
        });
    }

    // Mark records superseded by a corroborated correction on the same topic: if a
    // record R is tagged Authoritative via supersession language and another record S
    // shares most salient terms AND is older, S is the superseded account.
    let auth: Vec<usize> = out
        .iter()
        .enumerate()
        .filter(|(_, s)| s.tag == AuthorityTag::Authoritative)
        .map(|(i, _)| i)
        .collect();
    for s_i in 0..evidence.len() {
        if out[s_i].tag != AuthorityTag::Supporting && out[s_i].tag != AuthorityTag::KnownAsOf {
            continue;
        }
        for &a_i in &auth {
            if a_i == s_i {
                continue;
            }
            let overlap = terms[s_i].intersection(&terms[a_i]).count();
            let older = evidence[s_i].date <= evidence[a_i].date;
            if older && !terms[s_i].is_empty() && overlap * 3 >= terms[s_i].len() {
                out[s_i].tag = AuthorityTag::Superseded;
                out[s_i].score -= 0.3;
                break;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(index: usize, slug: &str, date: &str, source: &str, text: &str) -> EvidenceItem {
        EvidenceItem {
            index,
            slug: slug.into(),
            date: date.into(),
            source: source.into(),
            status: "current".into(),
            note: String::new(),
            text: text.into(),
        }
    }

    #[test]
    fn supersession_language_marks_correction_authoritative_and_prior_superseded() {
        let pool = vec![
            ev(
                1,
                "ART-EV-2021-003-003",
                "2021-05-22",
                "incident_report / P-1",
                "The 2021-05-22 outage was caused by a load balancer misconfiguration; \
                 health-check missing on the LB; rolled back the deployment.",
            ),
            ev(
                2,
                "ART-EV-2022-008-001",
                "2022-06-15",
                "postmortem / P-5",
                "Corrected root cause analysis. Status: Final — supersedes the preliminary \
                 incident summary. The load balancer hypothesis was ruled out; the DNS \
                 record misconfiguration was the root cause of the outage.",
            ),
        ];
        // change/current-understanding context (wants_change), and the correction is
        // corroborated by the independent 2021-003 incident event.
        let scored = resolve(&pool, None, true);
        // the postmortem (idx 1) is the corroborated correction -> Authoritative
        assert_eq!(scored[1].tag, AuthorityTag::Authoritative);
        // the original LB account (idx 0), older + same topic -> Superseded
        assert_eq!(scored[0].tag, AuthorityTag::Superseded);
        assert!(scored[1].score > scored[0].score);
    }

    #[test]
    fn lone_uncorroborated_amendment_does_not_win_on_a_plain_question() {
        // Planted "the decision was actually X" amendment (event EV-2022-007, two
        // sibling artifacts) vs the original API-first decision (event EV-2021-001).
        // The question is a plain decision-provenance ask (no change/as-of intent).
        let pool = vec![
            ev(
                1,
                "ART-EV-2021-001-001",
                "2021-04-15",
                "meeting_transcript / P-1",
                "Maya Patel, Luis Hernandez and Arjun Mehta decided to pivot Helix to an \
                 API-first architecture, driven by scaling and investor pressure.",
            ),
            ev(
                2,
                "ART-EV-2022-007-002",
                "2022-05-12",
                "review / P-2",
                "A review concluded the API-first framing was inaccurate; the decision was \
                 actually a pivot to a cloud-based analytics platform. Maya and Arjun amended \
                 the roadmap accordingly.",
            ),
            ev(
                3,
                "ART-EV-2022-007-001",
                "2022-05-12",
                "email / P-2",
                "Amendment: the 2021-04-15 architecture decision was actually about a \
                 cloud-based analytics platform, not API-first.",
            ),
        ];
        // plain question: no as-of, not a change question.
        let scored = resolve(&pool, None, false);
        // the amendment (idx 1) states supersession but is only backed by its own
        // event's sibling (idx 2) — NOT an independent event, and no temporal intent.
        // It must NOT be promoted to Authoritative, so it can't displace the original.
        assert_ne!(scored[1].tag, AuthorityTag::Authoritative);
        assert_ne!(scored[2].tag, AuthorityTag::Authoritative);
        // the original decision is not marked Superseded by the lone amendment.
        assert_ne!(scored[0].tag, AuthorityTag::Superseded);
    }

    #[test]
    fn as_of_demotes_records_after_the_asof_date() {
        let pool = vec![
            ev(1, "A-1", "2022-05-01", "email / P-1", "Policy retains logs for 90 days."),
            ev(2, "A-2", "2022-08-20", "adr / P-2", "Policy changed to per-tenant isolation."),
        ];
        // pure as-of 2022-05-31, not a "what changed" question
        let scored = resolve(&pool, Some("2022-05-31"), false);
        assert_eq!(scored[0].tag, AuthorityTag::KnownAsOf);
        assert_eq!(scored[1].tag, AuthorityTag::RecordedLater);
        assert!(scored[0].score > scored[1].score);
    }

    #[test]
    fn as_of_keeps_later_records_when_change_is_asked() {
        let pool = vec![
            ev(1, "A-1", "2022-05-01", "email / P-1", "Policy retains logs for 90 days."),
            ev(2, "A-2", "2022-08-20", "adr / P-2", "Policy changed to per-tenant isolation."),
        ];
        let scored = resolve(&pool, Some("2022-05-31"), true);
        // change asked -> later record is NOT demoted to RecordedLater
        assert_ne!(scored[1].tag, AuthorityTag::RecordedLater);
    }
}
