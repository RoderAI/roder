//! Dream-enrichment data shapes for bt-gbrain.

pub mod entities;
pub mod extract;
pub mod graph_layout;
pub mod invalidation;
pub mod ontology;
pub mod relevance;
pub mod scheduler;
pub mod types;

pub use entities::{
    AliasResolution, CanonicalEntity, CanonicalRelationship, EntityAlias, EntityKind,
    EntityMention, EntityMergeCandidate, RelationshipMention, canonical_entity_id,
    dedupe_relationship_mentions, detect_entity_merge_candidates, entity_mention, person_mention,
    relationship_mention, resolve_entity_alias,
};
pub use extract::{
    DreamStatement, ExtractionError, RawTextChunk, StatementExtraction, StatementExtractor,
    ValidatedQuoteSpan, validate_quote_span,
};
pub use graph_layout::{GraphIdParts, normalize_graph_id, validate_graph_edge_endpoints};
pub use invalidation::{
    DerivedEventCandidate, DreamLinkCandidate, DreamLinkKind, InvalidationDecision,
    propose_invalidation_links,
};
pub use ontology::{
    OntologyEdgeDef, OntologyNodeDef, PredicateFamilyDef, seed_ontology_edges, seed_ontology_nodes,
    seed_predicate_families,
};
pub use relevance::{
    CommunityIdInput, EvidenceCardScoreInput, score_evidence_card, stable_community_id,
};
pub use scheduler::{
    DreamScheduleConfig, ScheduledDreamOutcome, ScheduledDreamSkipReason, run_scheduled_dream_once,
    spawn_periodic_dream_scheduler,
};
pub use types::{
    ConfidenceLabel, DreamMode, DreamPolicy, DreamRun, DreamStatus, EvidenceCard, GraphEdge,
    GraphHyperedge, GraphNode, TemporalEvent,
};
