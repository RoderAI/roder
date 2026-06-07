use roder_ext_gbrain::dream::extract::validate_quote_span;
use roder_ext_gbrain::dream::invalidation::{
    DerivedEventCandidate, DreamLinkKind, InvalidationDecision, propose_invalidation_links,
};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

#[test]
fn explicit_policy_replacement_creates_invalidation_link() {
    let old = event(
        "event-old",
        "orgmem:policy:retention",
        "policy",
        "The retention policy is 30 days.",
        Some("2026-01-01T00:00:00Z"),
    );
    let new = event(
        "event-new",
        "orgmem:policy:retention",
        "policy",
        "New policy replaces the retention policy with 90 days.",
        Some("2026-02-01T00:00:00Z"),
    );

    let decisions = propose_invalidation_links(&[old], &[new]);
    let candidate = decisions
        .iter()
        .find_map(|decision| match decision {
            InvalidationDecision::Candidate(candidate) => Some(candidate),
            InvalidationDecision::Rejected(_) => None,
        })
        .expect("explicit replacement should produce a candidate");

    assert_eq!(candidate.kind, DreamLinkKind::Invalidates);
    assert_eq!(candidate.source_event_id, "event-new");
    assert_eq!(candidate.target_event_id, "event-old");
    assert_eq!(
        candidate.evidence_quote,
        "New policy replaces the retention policy with 90 days."
    );
}

#[test]
fn unsupported_conflict_does_not_become_resolved_contradiction() {
    let old = event(
        "event-old",
        "orgmem:system:api_gateway",
        "owns",
        "Maya owns the API Gateway.",
        Some("2026-01-01T00:00:00Z"),
    );
    let new = event(
        "event-new",
        "orgmem:system:api_gateway",
        "owns",
        "Alex owns the API Gateway.",
        Some("2026-02-01T00:00:00Z"),
    );

    let decisions = propose_invalidation_links(&[old], &[new]);

    assert!(
        decisions
            .iter()
            .all(|decision| !matches!(decision, InvalidationDecision::Candidate(_)))
    );
    assert!(decisions.iter().any(|decision| matches!(
        decision,
        InvalidationDecision::Rejected(reason) if reason.contains("no explicit resolution")
    )));
}

#[test]
fn fact_supersedes_metadata_creates_supersedes_candidate() {
    let old = event(
        "event-old",
        "orgmem:policy:access",
        "policy",
        "Contractors may access staging.",
        Some("2026-01-01T00:00:00Z"),
    );
    let mut new = event(
        "event-new",
        "orgmem:policy:access",
        "policy",
        "Contractor staging access requires manager approval.",
        Some("2026-03-01T00:00:00Z"),
    );
    new.supersedes_event_ids.push("event-old".to_string());

    let decisions = propose_invalidation_links(&[old], &[new]);

    assert!(decisions.iter().any(|decision| matches!(
        decision,
        InvalidationDecision::Candidate(candidate)
            if candidate.kind == DreamLinkKind::Supersedes
                && candidate.target_event_id == "event-old"
    )));
}

#[test]
fn chronologically_impossible_invalidation_is_rejected() {
    let old = event(
        "event-old",
        "orgmem:policy:retention",
        "policy",
        "The retention policy is 90 days.",
        Some("2026-03-01T00:00:00Z"),
    );
    let new = event(
        "event-new",
        "orgmem:policy:retention",
        "policy",
        "New policy replaces the retention policy with 30 days.",
        Some("2026-02-01T00:00:00Z"),
    );

    let decisions = propose_invalidation_links(&[old], &[new]);

    assert!(decisions.iter().any(|decision| matches!(
        decision,
        InvalidationDecision::Rejected(reason) if reason.contains("cannot invalidate newer event")
    )));
}

fn event(
    id: &str,
    entity_id: &str,
    predicate_family: &str,
    text: &str,
    valid_at: Option<&str>,
) -> DerivedEventCandidate {
    DerivedEventCandidate {
        id: id.to_string(),
        entity_ids: vec![entity_id.to_string()],
        predicate_family: predicate_family.to_string(),
        statement_text: text.to_string(),
        quote: validate_quote_span(text, 0, text.len(), text).unwrap(),
        valid_at: valid_at.map(|value| OffsetDateTime::parse(value, &Rfc3339).unwrap()),
        supersedes_event_ids: Vec::new(),
        correction_of_event_ids: Vec::new(),
    }
}
