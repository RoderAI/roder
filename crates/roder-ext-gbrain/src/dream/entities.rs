use serde::{Deserialize, Serialize};

use super::{GraphIdParts, normalize_graph_id};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Person,
    Team,
    Product,
    System,
    Policy,
    Incident,
    Decision,
    Meeting,
    Document,
    Artifact,
    Organization,
    Unknown,
}

impl EntityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::Team => "team",
            Self::Product => "product",
            Self::System => "system",
            Self::Policy => "policy",
            Self::Incident => "incident",
            Self::Decision => "decision",
            Self::Meeting => "meeting",
            Self::Document => "document",
            Self::Artifact => "artifact",
            Self::Organization => "organization",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityAlias {
    pub alias: String,
    pub normalized_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalEntity {
    pub id: String,
    pub kind: EntityKind,
    pub label: String,
    pub aliases: Vec<EntityAlias>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AliasResolution {
    pub canonical_id: String,
    pub matched_alias: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityMention {
    pub id: String,
    pub kind: EntityKind,
    pub label: String,
    pub relationship_keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_fact_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityMergeCandidate {
    pub canonical_id: String,
    pub duplicate_id: String,
    pub confidence_score: u8,
    pub reason: String,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationshipMention {
    pub source_entity_id: String,
    pub relation: String,
    pub target_entity_id: String,
    pub evidence_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalRelationship {
    pub source_entity_id: String,
    pub relation: String,
    pub target_entity_id: String,
    pub evidence_ids: Vec<String>,
}

pub fn canonical_entity_id(scope: Option<&str>, kind: EntityKind, label: &str) -> String {
    normalize_graph_id(GraphIdParts {
        scope,
        kind: kind.as_str(),
        label,
    })
}

impl CanonicalEntity {
    pub fn new(scope: Option<&str>, kind: EntityKind, label: impl Into<String>) -> Self {
        let label = label.into();
        Self {
            id: canonical_entity_id(scope, kind, &label),
            kind,
            label,
            aliases: Vec::new(),
        }
    }

    pub fn with_aliases(
        mut self,
        scope: Option<&str>,
        aliases: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        let mut aliases = aliases
            .into_iter()
            .map(|alias| {
                let alias = alias.into();
                EntityAlias {
                    normalized_id: canonical_entity_id(scope, self.kind, &alias),
                    alias,
                }
            })
            .collect::<Vec<_>>();
        aliases.sort_by(|a, b| {
            a.normalized_id
                .cmp(&b.normalized_id)
                .then_with(|| a.alias.cmp(&b.alias))
        });
        aliases.dedup_by(|a, b| a.normalized_id == b.normalized_id);
        self.aliases = aliases;
        self
    }
}

pub fn resolve_entity_alias(
    scope: Option<&str>,
    kind: EntityKind,
    label_or_alias: &str,
    entities: &[CanonicalEntity],
) -> Option<AliasResolution> {
    let normalized = canonical_entity_id(scope, kind, label_or_alias);
    let mut matches = entities
        .iter()
        .filter_map(|entity| {
            if entity.kind != kind {
                return None;
            }
            if entity.id == normalized {
                return Some(AliasResolution {
                    canonical_id: entity.id.clone(),
                    matched_alias: entity.label.clone(),
                });
            }
            entity.aliases.iter().find_map(|alias| {
                (alias.normalized_id == normalized).then(|| AliasResolution {
                    canonical_id: entity.id.clone(),
                    matched_alias: alias.alias.clone(),
                })
            })
        })
        .collect::<Vec<_>>();
    matches.sort_by(|a, b| a.canonical_id.cmp(&b.canonical_id));
    matches.into_iter().next()
}

pub fn person_mention(
    scope: Option<&str>,
    label: impl Into<String>,
    relationship_keys: impl IntoIterator<Item = impl Into<String>>,
) -> EntityMention {
    entity_mention(scope, EntityKind::Person, label, relationship_keys)
}

pub fn entity_mention(
    scope: Option<&str>,
    kind: EntityKind,
    label: impl Into<String>,
    relationship_keys: impl IntoIterator<Item = impl Into<String>>,
) -> EntityMention {
    let label = label.into();
    let mut relationship_keys = relationship_keys
        .into_iter()
        .map(Into::into)
        .map(|key: String| normalize_relationship_key(&key))
        .filter(|key| !key.is_empty())
        .collect::<Vec<_>>();
    relationship_keys.sort();
    relationship_keys.dedup();

    EntityMention {
        id: canonical_entity_id(scope, kind, &label),
        kind,
        label,
        relationship_keys,
        source_fact_id: None,
    }
}

pub fn relationship_mention(
    scope: Option<&str>,
    source_kind: EntityKind,
    source_label: impl Into<String>,
    relation: impl Into<String>,
    target_kind: EntityKind,
    target_label: impl Into<String>,
    evidence_id: impl Into<String>,
) -> RelationshipMention {
    let source_label = source_label.into();
    let target_label = target_label.into();
    RelationshipMention {
        source_entity_id: canonical_entity_id(scope, source_kind, &source_label),
        relation: normalize_relationship_key(&relation.into()),
        target_entity_id: canonical_entity_id(scope, target_kind, &target_label),
        evidence_id: evidence_id.into(),
    }
}

pub fn detect_entity_merge_candidates(mentions: &[EntityMention]) -> Vec<EntityMergeCandidate> {
    let mut candidates = Vec::new();
    for (idx, left) in mentions.iter().enumerate() {
        for right in mentions.iter().skip(idx + 1) {
            if left.kind != right.kind || left.id == right.id {
                continue;
            }
            if let Some(candidate) = detect_merge_candidate(left, right) {
                candidates.push(candidate);
            }
        }
    }
    candidates.sort_by(|a, b| {
        a.canonical_id
            .cmp(&b.canonical_id)
            .then_with(|| a.duplicate_id.cmp(&b.duplicate_id))
    });
    candidates
        .dedup_by(|a, b| a.canonical_id == b.canonical_id && a.duplicate_id == b.duplicate_id);
    candidates
}

pub fn dedupe_relationship_mentions(
    relationships: &[RelationshipMention],
    merge_candidates: &[EntityMergeCandidate],
) -> Vec<CanonicalRelationship> {
    let mut remaps = std::collections::HashMap::new();
    for candidate in merge_candidates {
        remaps.insert(
            candidate.duplicate_id.clone(),
            candidate.canonical_id.clone(),
        );
    }

    let mut grouped: std::collections::BTreeMap<(String, String, String), Vec<String>> =
        std::collections::BTreeMap::new();
    for relationship in relationships {
        let source = remaps
            .get(&relationship.source_entity_id)
            .unwrap_or(&relationship.source_entity_id)
            .clone();
        let target = remaps
            .get(&relationship.target_entity_id)
            .unwrap_or(&relationship.target_entity_id)
            .clone();
        grouped
            .entry((source, relationship.relation.clone(), target))
            .or_default()
            .push(relationship.evidence_id.clone());
    }

    grouped
        .into_iter()
        .map(
            |((source_entity_id, relation, target_entity_id), mut evidence_ids)| {
                evidence_ids.sort();
                evidence_ids.dedup();
                CanonicalRelationship {
                    source_entity_id,
                    relation,
                    target_entity_id,
                    evidence_ids,
                }
            },
        )
        .collect()
}

fn detect_merge_candidate(
    left: &EntityMention,
    right: &EntityMention,
) -> Option<EntityMergeCandidate> {
    let left_name = ParsedPersonName::parse(&left.label)?;
    let right_name = ParsedPersonName::parse(&right.label)?;
    let same_name = left_name.normalized == right_name.normalized;
    let nickname_match = left.kind == EntityKind::Person && left_name.matches_alias(&right_name);
    let shared_relationships = shared_relationship_keys(left, right);

    if !same_name && !(nickname_match && !shared_relationships.is_empty()) {
        return None;
    }

    let (canonical, duplicate) = choose_canonical(left, right);
    let mut evidence = shared_relationships;
    if evidence.is_empty() {
        evidence.push("exact_name_match".to_string());
    }
    let confidence_score = if same_name { 100 } else { 82 };
    let reason = if same_name {
        "normalized entity labels match".to_string()
    } else {
        format!(
            "person names {} and {} are known aliases and share relationship context",
            left.label, right.label
        )
    };

    Some(EntityMergeCandidate {
        canonical_id: canonical.id.clone(),
        duplicate_id: duplicate.id.clone(),
        confidence_score,
        reason,
        evidence,
    })
}

fn choose_canonical<'a>(
    left: &'a EntityMention,
    right: &'a EntityMention,
) -> (&'a EntityMention, &'a EntityMention) {
    let left_rank = canonical_rank(left);
    let right_rank = canonical_rank(right);
    if left_rank >= right_rank {
        (left, right)
    } else {
        (right, left)
    }
}

fn canonical_rank(mention: &EntityMention) -> (usize, usize, std::cmp::Reverse<&str>) {
    let has_space = usize::from(mention.label.split_whitespace().count() > 1);
    (
        has_space,
        mention
            .label
            .chars()
            .filter(|ch| ch.is_alphabetic())
            .count(),
        std::cmp::Reverse(mention.id.as_str()),
    )
}

fn shared_relationship_keys(left: &EntityMention, right: &EntityMention) -> Vec<String> {
    let right_keys = right
        .relationship_keys
        .iter()
        .collect::<std::collections::BTreeSet<_>>();
    left.relationship_keys
        .iter()
        .filter(|key| right_keys.contains(key))
        .cloned()
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedPersonName {
    normalized: String,
    given: String,
    family: Option<String>,
}

impl ParsedPersonName {
    fn parse(label: &str) -> Option<Self> {
        let words = label
            .split(|ch: char| !(ch.is_alphanumeric() || ch == '\''))
            .filter(|part| !part.is_empty())
            .map(|part| part.to_ascii_lowercase())
            .collect::<Vec<_>>();
        let given = words.first()?.clone();
        let family = words.get(1).cloned();
        Some(Self {
            normalized: words.join(" "),
            given,
            family,
        })
    }

    fn matches_alias(&self, other: &Self) -> bool {
        if self.given == other.given {
            return family_compatible(self, other);
        }
        given_name_family(&self.given)
            .zip(given_name_family(&other.given))
            .is_some_and(|(left_family, right_family)| {
                left_family == right_family && family_compatible(self, other)
            })
    }
}

fn family_compatible(left: &ParsedPersonName, right: &ParsedPersonName) -> bool {
    match (&left.family, &right.family) {
        (Some(left_family), Some(right_family)) => left_family == right_family,
        _ => true,
    }
}

fn given_name_family(name: &str) -> Option<&'static str> {
    match name {
        "dan" | "danny" | "daniel" => Some("daniel"),
        "alex" | "alexander" | "alexandra" => Some("alex"),
        "liz" | "beth" | "elizabeth" => Some("elizabeth"),
        "mike" | "michael" => Some("michael"),
        "nick" | "nicholas" => Some("nicholas"),
        "sam" | "samantha" | "samuel" => Some("sam"),
        "tom" | "thomas" => Some("thomas"),
        "will" | "bill" | "william" => Some("william"),
        _ => None,
    }
}

fn normalize_relationship_key(key: &str) -> String {
    normalize_graph_id(GraphIdParts {
        scope: None,
        kind: "rel",
        label: key,
    })
}
