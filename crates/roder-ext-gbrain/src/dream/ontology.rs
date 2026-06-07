use serde::{Deserialize, Serialize};

use super::{ConfidenceLabel, GraphIdParts, normalize_graph_id};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OntologyNodeDef {
    pub id: String,
    pub label: String,
    pub node_kind: String,
    pub explanation: String,
    pub confidence: ConfidenceLabel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OntologyEdgeDef {
    pub id: String,
    pub source_node_id: String,
    pub target_node_id: String,
    pub relation: String,
    pub explanation: String,
    pub evidence_type: String,
    pub confidence: ConfidenceLabel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PredicateFamilyDef {
    pub id: String,
    pub label: String,
    pub volatility: String,
    pub examples: Vec<String>,
}

pub fn seed_ontology_nodes() -> Vec<OntologyNodeDef> {
    [
        ("person", "People", "Human actors and stakeholders"),
        (
            "team",
            "Teams",
            "Groups with shared responsibility or reporting context",
        ),
        (
            "product",
            "Products",
            "Product surfaces, shipped capabilities, or offerings",
        ),
        (
            "system",
            "Systems",
            "Technical systems, tools, services, and infrastructure",
        ),
        (
            "vendor",
            "Vendors",
            "External suppliers or service providers",
        ),
        (
            "customer",
            "Customers",
            "Customer organizations or named customer segments",
        ),
        (
            "policy",
            "Policies",
            "Rules, SLAs, access controls, and operational policies",
        ),
        (
            "incident",
            "Incidents",
            "Outages, regressions, escalations, and remediation windows",
        ),
        (
            "decision",
            "Decisions",
            "Chosen options with actors, alternatives, and rationale",
        ),
        (
            "meeting",
            "Meetings",
            "Time-bound discussions with participants and outcomes",
        ),
        (
            "document",
            "Documents",
            "Docs, notes, plans, PRDs, tickets, and written artifacts",
        ),
        (
            "artifact",
            "Artifacts",
            "Source records, files, logs, imports, and evidence material",
        ),
    ]
    .into_iter()
    .map(|(kind, label, explanation)| OntologyNodeDef {
        id: ontology_id("node", kind),
        label: label.to_string(),
        node_kind: kind.to_string(),
        explanation: explanation.to_string(),
        confidence: ConfidenceLabel::Extracted,
    })
    .collect()
}

pub fn seed_ontology_edges() -> Vec<OntologyEdgeDef> {
    [
        (
            "person",
            "team",
            "reports_to",
            "Reporting questions should traverse from people to teams.",
        ),
        (
            "team",
            "product",
            "owns",
            "Ownership questions often bind teams to products.",
        ),
        (
            "team",
            "system",
            "owns",
            "Operational responsibility often binds teams to systems.",
        ),
        (
            "person",
            "decision",
            "decides",
            "Decision provenance requires actor, date, options, and rationale.",
        ),
        (
            "decision",
            "document",
            "justifies",
            "Decision rationale is usually preserved in documents.",
        ),
        (
            "policy",
            "policy",
            "supersedes",
            "Policy history requires predecessor and replacement links.",
        ),
        (
            "policy",
            "incident",
            "impacts",
            "Incidents can trigger policy or SLA changes.",
        ),
        (
            "system",
            "incident",
            "impacts",
            "Incident replay should traverse affected systems.",
        ),
        (
            "document",
            "artifact",
            "mentions",
            "Documents and source artifacts preserve quote-backed evidence.",
        ),
        (
            "decision",
            "system",
            "implements",
            "Implementation questions connect decisions to systems.",
        ),
        (
            "artifact",
            "policy",
            "invalidates",
            "Source artifacts may explicitly invalidate policy state.",
        ),
        (
            "artifact",
            "decision",
            "contradicts",
            "Conflicting source artifacts must remain inspectable.",
        ),
    ]
    .into_iter()
    .map(|(source, target, relation, explanation)| OntologyEdgeDef {
        id: format!(
            "ontology_edge:{}:{}:{}",
            relation,
            ontology_id("node", source),
            ontology_id("node", target)
        ),
        source_node_id: ontology_id("node", source),
        target_node_id: ontology_id("node", target),
        relation: relation.to_string(),
        explanation: explanation.to_string(),
        evidence_type: "quote_backed_or_deterministic_source_structure".to_string(),
        confidence: ConfidenceLabel::Extracted,
    })
    .collect()
}

pub fn seed_predicate_families() -> Vec<PredicateFamilyDef> {
    [
        (
            "owns",
            "Ownership",
            "dynamic",
            vec!["owns", "responsible for", "DRI"],
        ),
        (
            "reports_to",
            "Reporting",
            "dynamic",
            vec!["reports to", "manager"],
        ),
        (
            "authorizes",
            "Authorization",
            "dynamic",
            vec!["approves", "authorizes"],
        ),
        ("decides", "Decision", "atemporal", vec!["decided", "chose"]),
        (
            "supersedes",
            "Supersession",
            "dynamic",
            vec!["replaces", "supersedes"],
        ),
        (
            "invalidates",
            "Invalidation",
            "dynamic",
            vec!["invalidates", "no longer applies"],
        ),
        (
            "contradicts",
            "Contradiction",
            "dynamic",
            vec!["contradicts", "conflicts with"],
        ),
        (
            "justifies",
            "Justification",
            "atemporal",
            vec!["because", "rationale"],
        ),
        (
            "implements",
            "Implementation",
            "dynamic",
            vec!["implements", "ships"],
        ),
        ("impacts", "Impact", "dynamic", vec!["affects", "impacts"]),
        (
            "mentions",
            "Mention",
            "static",
            vec!["mentions", "references"],
        ),
    ]
    .into_iter()
    .map(|(id, label, volatility, examples)| PredicateFamilyDef {
        id: id.to_string(),
        label: label.to_string(),
        volatility: volatility.to_string(),
        examples: examples.into_iter().map(str::to_string).collect(),
    })
    .collect()
}

fn ontology_id(kind: &str, label: &str) -> String {
    normalize_graph_id(GraphIdParts {
        scope: Some("ontology"),
        kind,
        label,
    })
}
