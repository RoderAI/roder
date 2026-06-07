use roder_ext_gbrain::{
    ClaimConfidence, ClaimTemporalScope, ClaimType, EvidenceRecord, LedgerClaim, QuoteSpan,
    validate_claim_ledger,
};

fn evidence() -> Vec<EvidenceRecord> {
    vec![
        EvidenceRecord::new(
            1,
            "ART-EV-2023-009-001",
            "Acme account owner is Maya Patel. The decision was recorded on 2023-03-12.",
        )
        .with_date("2023-03-12")
        .with_status("current"),
        EvidenceRecord::new(
            2,
            "ART-EV-2023-009-002",
            "The ownership transfer was unchanged after the March review.",
        )
        .with_date("2023-04-01")
        .with_status("current"),
    ]
}

fn claim(
    text: &str,
    claim_type: ClaimType,
    artifacts: Vec<&str>,
    records: Vec<usize>,
    quotes: Vec<QuoteSpan>,
) -> LedgerClaim {
    LedgerClaim {
        claim_id: "c1".to_string(),
        claim_text: text.to_string(),
        claim_type,
        supporting_artifact_ids: artifacts.into_iter().map(str::to_string).collect(),
        supporting_record_numbers: records,
        quote_spans: quotes,
        temporal_scope: ClaimTemporalScope::Unknown,
        confidence: ClaimConfidence::Rejected,
        rejection_reason: None,
    }
}

fn quote(record_number: usize, artifact_id: &str, text: &str) -> QuoteSpan {
    QuoteSpan {
        artifact_id: artifact_id.to_string(),
        record_number,
        quote: text.to_string(),
    }
}

#[test]
fn accepts_direct_claim_with_existing_record_quote_and_specifics() {
    let claim = claim(
        "Maya Patel owns Acme as of 2023-03-12 [ART-EV-2023-009-001].",
        ClaimType::Direct,
        vec!["ART-EV-2023-009-001"],
        vec![1],
        vec![quote(
            1,
            "ART-EV-2023-009-001",
            "Acme account owner is Maya Patel",
        )],
    );

    let trace = validate_claim_ledger(&[claim], &evidence());
    assert!(trace.is_fully_verified(), "{trace:#?}");
    assert_eq!(trace.verified[0].confidence, ClaimConfidence::Proven);
}

#[test]
fn rejects_claim_without_quote_span() {
    let claim = claim(
        "Maya Patel owns Acme.",
        ClaimType::Direct,
        vec!["ART-EV-2023-009-001"],
        vec![1],
        vec![],
    );

    let trace = validate_claim_ledger(&[claim], &evidence());
    assert_eq!(trace.rejected.len(), 1);
    assert!(
        trace
            .failures
            .iter()
            .any(|failure| failure.reason.contains("no quote")),
        "{trace:#?}"
    );
}

#[test]
fn rejects_specific_not_present_in_cited_evidence() {
    let claim = claim(
        "Maya Patel owns Acme and the retention risk was 70%.",
        ClaimType::Direct,
        vec!["ART-EV-2023-009-001"],
        vec![1],
        vec![quote(
            1,
            "ART-EV-2023-009-001",
            "Acme account owner is Maya Patel",
        )],
    );

    let trace = validate_claim_ledger(&[claim], &evidence());
    assert_eq!(trace.rejected.len(), 1);
    assert!(
        trace
            .failures
            .iter()
            .any(|failure| failure.reason.contains("70%")),
        "{trace:#?}"
    );
}

#[test]
fn rejects_cross_record_claim_marked_direct() {
    let claim = claim(
        "Maya Patel owns Acme and the ownership transfer was unchanged.",
        ClaimType::Direct,
        vec!["ART-EV-2023-009-001", "ART-EV-2023-009-002"],
        vec![1, 2],
        vec![
            quote(1, "ART-EV-2023-009-001", "Acme account owner is Maya Patel"),
            quote(2, "ART-EV-2023-009-002", "ownership transfer was unchanged"),
        ],
    );

    let trace = validate_claim_ledger(&[claim], &evidence());
    assert_eq!(trace.rejected.len(), 1);
    assert!(
        trace
            .failures
            .iter()
            .any(|failure| failure.reason.contains("cross-record")),
        "{trace:#?}"
    );
}

#[test]
fn accepts_cross_record_claim_marked_derived() {
    let claim = claim(
        "Maya Patel owns Acme and the ownership transfer was unchanged.",
        ClaimType::Derived,
        vec!["ART-EV-2023-009-001", "ART-EV-2023-009-002"],
        vec![1, 2],
        vec![
            quote(1, "ART-EV-2023-009-001", "Acme account owner is Maya Patel"),
            quote(2, "ART-EV-2023-009-002", "ownership transfer was unchanged"),
        ],
    );

    let trace = validate_claim_ledger(&[claim], &evidence());
    assert!(trace.is_fully_verified(), "{trace:#?}");
    assert_eq!(trace.verified[0].confidence, ClaimConfidence::Inferred);
}

#[test]
fn rejects_since_as_of_claim_without_explicit_change_support() {
    let mut claim = claim(
        "Maya Patel changed the Acme ownership after 2023-03-12.",
        ClaimType::TemporalStatus,
        vec!["ART-EV-2023-009-001"],
        vec![1],
        vec![quote(
            1,
            "ART-EV-2023-009-001",
            "Acme account owner is Maya Patel",
        )],
    );
    claim.temporal_scope = ClaimTemporalScope::SinceAsOf;

    let trace = validate_claim_ledger(&[claim], &evidence());
    assert_eq!(trace.rejected.len(), 1);
    assert!(
        trace
            .failures
            .iter()
            .any(|failure| failure.reason.contains("since_as_of")),
        "{trace:#?}"
    );
}
