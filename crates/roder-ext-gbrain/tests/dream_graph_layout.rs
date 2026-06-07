use roder_ext_gbrain::{GraphIdParts, normalize_graph_id, validate_graph_edge_endpoints};

#[test]
fn graph_id_normalization_is_scope_aware_and_stable() {
    let id = normalize_graph_id(GraphIdParts {
        scope: Some("Repo: Gode"),
        kind: "Person",
        label: "Maya Patel / CTO",
    });

    assert_eq!(id, "repo_gode:person:maya_patel_cto");
}

#[test]
fn graph_id_normalization_applies_nfkc_casefolding_and_punctuation_rules() {
    let id = normalize_graph_id(GraphIdParts {
        scope: Some("Import"),
        kind: "System",
        label: "ＡＣＭＥ–Router",
    });

    assert_eq!(id, "import:system:acme_router");
}

#[test]
fn graph_edge_endpoint_validation_rejects_empty_and_self_edges() {
    assert!(validate_graph_edge_endpoints("node:a", "node:b").is_ok());
    assert!(validate_graph_edge_endpoints("", "node:b").is_err());
    assert!(validate_graph_edge_endpoints("node:a", "").is_err());
    assert!(validate_graph_edge_endpoints("node:a", "node:a").is_err());
}
