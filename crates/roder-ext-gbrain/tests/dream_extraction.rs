use roder_ext_gbrain::ConfidenceLabel;
use roder_ext_gbrain::dream::entities::{
    CanonicalEntity, EntityKind, canonical_entity_id, dedupe_relationship_mentions,
    detect_entity_merge_candidates, person_mention, relationship_mention, resolve_entity_alias,
};
use roder_ext_gbrain::dream::extract::{
    DeterministicStatementExtractor, RawTextChunk, StatementExtractor, validate_quote_span,
};
use roder_ext_gbrain::dream::ontology::{
    seed_ontology_edges, seed_ontology_nodes, seed_predicate_families,
};
use roder_ext_gbrain::dream::relevance::{CommunityIdInput, stable_community_id};

#[test]
fn quote_span_validation_rejects_missing_text() {
    let raw = "The platform SLA is 99.9%.";
    let err = validate_quote_span(raw, 0, 12, "The policy").unwrap_err();

    assert!(err.to_string().contains("quote span text mismatch"));
}

#[test]
fn deterministic_extraction_returns_multiple_quote_backed_statements() {
    let chunk = RawTextChunk {
        source_fact_id: "fact-1".to_string(),
        artifact_slug: "policies/platform.md".to_string(),
        text: "The platform SLA is 99.9%. Maya owns the API Gateway today.".to_string(),
    };

    let extraction = DeterministicStatementExtractor.extract(&chunk).unwrap();

    assert_eq!(extraction.statements.len(), 2);
    assert_eq!(extraction.statements[0].source_fact_id, "fact-1");
    assert_eq!(
        extraction.statements[0].quote.text,
        "The platform SLA is 99.9%."
    );
    assert_eq!(
        extraction.statements[1].artifact_slug,
        "policies/platform.md"
    );
    assert_eq!(
        extraction.statements[1].confidence,
        ConfidenceLabel::Extracted
    );
    assert!(
        extraction.statements[1]
            .temporal_cues
            .contains(&"today".to_string())
    );
}

#[test]
fn alias_resolution_is_stable_for_canonical_entities() {
    let entity = CanonicalEntity::new(Some("orgmem"), EntityKind::Person, "Maya Patel")
        .with_aliases(Some("orgmem"), ["M. Patel", "maya"]);

    let direct = canonical_entity_id(Some("orgmem"), EntityKind::Person, "Maya Patel");
    let alias = resolve_entity_alias(Some("orgmem"), EntityKind::Person, "M. Patel", &[entity])
        .expect("alias should resolve");

    assert_eq!(direct, "orgmem:person:maya_patel");
    assert_eq!(alias.canonical_id, direct);
    assert_eq!(alias.matched_alias, "M. Patel");
}

#[test]
fn person_dedupe_detects_dan_and_daniel_from_shared_relationship_context() {
    let mentions = vec![
        person_mention(Some("orgmem"), "Dan", ["team:platform", "owns:api"]),
        person_mention(Some("orgmem"), "Daniel", ["team:platform", "owns:api"]),
        person_mention(Some("orgmem"), "Dana", ["team:sales"]),
    ];

    let candidates = detect_entity_merge_candidates(&mentions);

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].canonical_id, "orgmem:person:daniel");
    assert_eq!(candidates[0].duplicate_id, "orgmem:person:dan");
    assert!(candidates[0].reason.contains("known aliases"));
    assert!(
        candidates[0]
            .evidence
            .iter()
            .any(|item| item.ends_with("team_platform"))
    );
}

#[test]
fn relationship_dedupe_remaps_alias_entities_before_grouping_edges() {
    let mentions = vec![
        person_mention(
            Some("orgmem"),
            "Dan",
            ["team:platform", "system:api_gateway"],
        ),
        person_mention(
            Some("orgmem"),
            "Daniel",
            ["team:platform", "system:api_gateway"],
        ),
    ];
    let merges = detect_entity_merge_candidates(&mentions);
    let relationships = vec![
        relationship_mention(
            Some("orgmem"),
            EntityKind::Person,
            "Dan",
            "owns",
            EntityKind::System,
            "API Gateway",
            "fact-1",
        ),
        relationship_mention(
            Some("orgmem"),
            EntityKind::Person,
            "Daniel",
            "owns",
            EntityKind::System,
            "API Gateway",
            "fact-2",
        ),
    ];

    let deduped = dedupe_relationship_mentions(&relationships, &merges);

    assert_eq!(deduped.len(), 1);
    assert_eq!(deduped[0].source_entity_id, "orgmem:person:daniel");
    assert_eq!(deduped[0].target_entity_id, "orgmem:system:api_gateway");
    assert_eq!(deduped[0].evidence_ids, vec!["fact-1", "fact-2"]);
}

#[test]
fn seeded_ontology_contains_required_memory_shapes_and_relations() {
    let nodes = seed_ontology_nodes();
    let node_kinds = nodes
        .iter()
        .map(|node| node.node_kind.as_str())
        .collect::<Vec<_>>();
    for required in [
        "person", "team", "product", "system", "policy", "incident", "decision", "meeting",
        "document", "artifact",
    ] {
        assert!(
            node_kinds.contains(&required),
            "missing ontology node {required}"
        );
    }

    let edges = seed_ontology_edges();
    assert!(edges.iter().any(|edge| edge.relation == "supersedes"));
    assert!(edges.iter().any(|edge| edge.relation == "invalidates"));
    assert!(edges.iter().any(|edge| edge.relation == "contradicts"));

    let predicates = seed_predicate_families();
    assert!(predicates.iter().any(|predicate| predicate.id == "owns"));
    assert!(
        predicates
            .iter()
            .any(|predicate| predicate.id == "reports_to")
    );
    assert!(
        predicates
            .iter()
            .any(|predicate| predicate.id == "justifies")
    );
}

#[test]
fn community_ids_are_stable_for_node_set_ordering() {
    let first = stable_community_id(CommunityIdInput {
        scope_id: "orgmem".to_string(),
        algorithm_version: "dream-v1".to_string(),
        node_ids: vec![
            "orgmem:person:maya".to_string(),
            "orgmem:system:api_gateway".to_string(),
            "orgmem:policy:sla".to_string(),
        ],
    });
    let second = stable_community_id(CommunityIdInput {
        scope_id: "orgmem".to_string(),
        algorithm_version: "dream-v1".to_string(),
        node_ids: vec![
            "orgmem:policy:sla".to_string(),
            "orgmem:person:maya".to_string(),
            "orgmem:system:api_gateway".to_string(),
            "orgmem:policy:sla".to_string(),
        ],
    });

    assert_eq!(first, second);
    assert!(first.starts_with("community:orgmem:"));
}
