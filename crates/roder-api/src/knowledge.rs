//! Project knowledge base contracts (roadmap phase 93).
//!
//! Knowledge documents are larger, titled, kind-tagged artifacts (requirements,
//! decisions, research, runbooks, ...) with revisions and typed links — distinct
//! from atomic memory records. Stores are provided by extensions; the first
//! engine is the markdown-file-based `roder-ext-knowledge-md`.

use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::extension::KnowledgeStoreId;
use crate::memory::MemoryScope;

pub type KnowledgeDocId = String;

/// Document kind. Extensible: unknown kinds round-trip through `Other`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KnowledgeKind {
    Memory,
    Requirement,
    Decision,
    Research,
    Runbook,
    Artifact,
    Note,
    Other(String),
}

impl KnowledgeKind {
    pub const KNOWN: [&'static str; 7] = [
        "memory",
        "requirement",
        "decision",
        "research",
        "runbook",
        "artifact",
        "note",
    ];

    pub fn as_str(&self) -> &str {
        match self {
            KnowledgeKind::Memory => "memory",
            KnowledgeKind::Requirement => "requirement",
            KnowledgeKind::Decision => "decision",
            KnowledgeKind::Research => "research",
            KnowledgeKind::Runbook => "runbook",
            KnowledgeKind::Artifact => "artifact",
            KnowledgeKind::Note => "note",
            KnowledgeKind::Other(value) => value,
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "memory" => KnowledgeKind::Memory,
            "requirement" => KnowledgeKind::Requirement,
            "decision" => KnowledgeKind::Decision,
            "research" => KnowledgeKind::Research,
            "runbook" => KnowledgeKind::Runbook,
            "artifact" => KnowledgeKind::Artifact,
            "note" => KnowledgeKind::Note,
            other => KnowledgeKind::Other(other.to_string()),
        }
    }
}

impl fmt::Display for KnowledgeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for KnowledgeKind {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for KnowledgeKind {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Ok(KnowledgeKind::parse(&value))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeStatus {
    Active,
    Draft,
    Superseded,
    Archived,
}

impl KnowledgeStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            KnowledgeStatus::Active => "active",
            KnowledgeStatus::Draft => "draft",
            KnowledgeStatus::Superseded => "superseded",
            KnowledgeStatus::Archived => "archived",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeSource {
    User,
    Agent,
    Reconciler,
    Import,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeLinkType {
    RelatesTo,
    Supersedes,
    DerivedFrom,
    Contradicts,
    Duplicates,
}

impl KnowledgeLinkType {
    pub fn as_str(&self) -> &'static str {
        match self {
            KnowledgeLinkType::RelatesTo => "relates_to",
            KnowledgeLinkType::Supersedes => "supersedes",
            KnowledgeLinkType::DerivedFrom => "derived_from",
            KnowledgeLinkType::Contradicts => "contradicts",
            KnowledgeLinkType::Duplicates => "duplicates",
        }
    }
}

/// Typed edge from the owning document to `to`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeLink {
    #[serde(rename = "type")]
    pub link_type: KnowledgeLinkType,
    pub to: KnowledgeDocId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeDocument {
    pub id: KnowledgeDocId,
    pub scope: MemoryScope,
    pub kind: KnowledgeKind,
    pub slug: String,
    pub title: String,
    pub status: KnowledgeStatus,
    pub source: KnowledgeSource,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub links: Vec<KnowledgeLink>,
    pub revision: u32,
    pub content_hash: String,
    pub body: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

impl KnowledgeDocument {
    pub fn summary(&self) -> KnowledgeDocSummary {
        const PREVIEW_CHARS: usize = 160;
        let preview = if self.body.chars().count() <= PREVIEW_CHARS {
            self.body.clone()
        } else {
            let mut out = self.body.chars().take(PREVIEW_CHARS).collect::<String>();
            out.push_str("...");
            out
        };
        KnowledgeDocSummary {
            id: self.id.clone(),
            scope: self.scope.clone(),
            kind: self.kind.clone(),
            slug: self.slug.clone(),
            title: self.title.clone(),
            status: self.status,
            source: self.source,
            tags: self.tags.clone(),
            links: self.links.clone(),
            revision: self.revision,
            byte_count: self.body.len() as u64,
            preview,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

/// Body-free view used for listing and events so corpora stay cheap to ship.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeDocSummary {
    pub id: KnowledgeDocId,
    pub scope: MemoryScope,
    pub kind: KnowledgeKind,
    pub slug: String,
    pub title: String,
    pub status: KnowledgeStatus,
    pub source: KnowledgeSource,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub links: Vec<KnowledgeLink>,
    pub revision: u32,
    pub byte_count: u64,
    pub preview: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeListQuery {
    pub scope: Option<MemoryScope>,
    #[serde(default)]
    pub kind: Option<KnowledgeKind>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub status: Option<KnowledgeStatus>,
    #[serde(default)]
    pub include_archived: bool,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeQuery {
    pub scope: Option<MemoryScope>,
    pub text: String,
    #[serde(default)]
    pub kind: Option<KnowledgeKind>,
    pub limit: usize,
    #[serde(default)]
    pub include_global: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeCitation {
    pub doc_id: KnowledgeDocId,
    pub scope_id: String,
    pub title: String,
    pub snippet: String,
    pub score_millis: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeSearchResult {
    pub document: KnowledgeDocSummary,
    pub score: f32,
    pub snippet: String,
    pub citation: KnowledgeCitation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeSaveRequest {
    pub scope: MemoryScope,
    pub kind: KnowledgeKind,
    pub title: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub body: String,
    pub source: KnowledgeSource,
}

/// Partial update; absent fields keep the current value. Every applied
/// update writes a new immutable revision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeUpdateRequest {
    pub id: KnowledgeDocId,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub status: Option<KnowledgeStatus>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    pub source: KnowledgeSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeLinkRequest {
    pub from: KnowledgeDocId,
    pub to: KnowledgeDocId,
    #[serde(rename = "type")]
    pub link_type: KnowledgeLinkType,
    #[serde(default)]
    pub remove: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeRevisionInfo {
    pub revision: u32,
    pub content_hash: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

#[async_trait::async_trait]
pub trait KnowledgeStore: Send + Sync {
    fn id(&self) -> KnowledgeStoreId;

    async fn save(&self, request: KnowledgeSaveRequest) -> anyhow::Result<KnowledgeDocument>;
    async fn get(&self, id: &KnowledgeDocId) -> anyhow::Result<Option<KnowledgeDocument>>;
    async fn get_revision(
        &self,
        id: &KnowledgeDocId,
        revision: u32,
    ) -> anyhow::Result<Option<KnowledgeDocument>>;
    async fn list(&self, query: KnowledgeListQuery) -> anyhow::Result<Vec<KnowledgeDocSummary>>;
    async fn search(&self, query: KnowledgeQuery) -> anyhow::Result<Vec<KnowledgeSearchResult>>;
    async fn update(&self, request: KnowledgeUpdateRequest) -> anyhow::Result<KnowledgeDocument>;
    /// Soft-delete: the document leaves default lists but stays readable by id.
    async fn archive(&self, id: &KnowledgeDocId) -> anyhow::Result<bool>;
    async fn set_link(&self, request: KnowledgeLinkRequest) -> anyhow::Result<KnowledgeDocument>;
    async fn revisions(&self, id: &KnowledgeDocId) -> anyhow::Result<Vec<KnowledgeRevisionInfo>>;
}

pub trait KnowledgeStoreFactory: Send + Sync + 'static {
    fn id(&self) -> KnowledgeStoreId;
    fn create(&self) -> Arc<dyn KnowledgeStore>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knowledge_kind_serializes_as_plain_string() {
        assert_eq!(
            serde_json::to_value(KnowledgeKind::Decision).unwrap(),
            serde_json::json!("decision")
        );
        assert_eq!(
            serde_json::to_value(KnowledgeKind::Other("postmortem".to_string())).unwrap(),
            serde_json::json!("postmortem")
        );
    }

    #[test]
    fn knowledge_kind_round_trips_known_and_custom_values() {
        for kind in KnowledgeKind::KNOWN {
            let parsed = KnowledgeKind::parse(kind);
            assert_eq!(parsed.as_str(), kind);
            assert!(!matches!(parsed, KnowledgeKind::Other(_)));
        }
        assert_eq!(
            KnowledgeKind::parse("postmortem"),
            KnowledgeKind::Other("postmortem".to_string())
        );
    }

    #[test]
    fn knowledge_link_serializes_type_field() {
        let link = KnowledgeLink {
            link_type: KnowledgeLinkType::Supersedes,
            to: "kn-1".to_string(),
        };
        assert_eq!(
            serde_json::to_value(&link).unwrap(),
            serde_json::json!({ "type": "supersedes", "to": "kn-1" })
        );
    }

    #[test]
    fn document_summary_bounds_preview_and_keeps_metadata() {
        let now = OffsetDateTime::UNIX_EPOCH;
        let doc = KnowledgeDocument {
            id: "kn-1".to_string(),
            scope: MemoryScope::Project("p".to_string()),
            kind: KnowledgeKind::Research,
            slug: "long-doc".to_string(),
            title: "Long doc".to_string(),
            status: KnowledgeStatus::Active,
            source: KnowledgeSource::Agent,
            tags: vec!["api".to_string()],
            links: Vec::new(),
            revision: 3,
            content_hash: "hash".to_string(),
            body: "x".repeat(4000),
            created_at: now,
            updated_at: now,
        };

        let summary = doc.summary();

        assert_eq!(summary.byte_count, 4000);
        assert!(summary.preview.ends_with("..."));
        assert!(summary.preview.len() < 200);
        assert_eq!(summary.revision, 3);
        assert_eq!(summary.tags, vec!["api".to_string()]);
    }

    #[test]
    fn document_round_trips_json() {
        let now = OffsetDateTime::UNIX_EPOCH;
        let doc = KnowledgeDocument {
            id: "kn-2".to_string(),
            scope: MemoryScope::Global,
            kind: KnowledgeKind::Requirement,
            slug: "auth-req".to_string(),
            title: "Auth requirements".to_string(),
            status: KnowledgeStatus::Draft,
            source: KnowledgeSource::User,
            tags: vec!["auth".to_string()],
            links: vec![KnowledgeLink {
                link_type: KnowledgeLinkType::RelatesTo,
                to: "kn-1".to_string(),
            }],
            revision: 1,
            content_hash: "h".to_string(),
            body: "Users must log in.".to_string(),
            created_at: now,
            updated_at: now,
        };

        let value = serde_json::to_value(&doc).unwrap();
        let decoded: KnowledgeDocument = serde_json::from_value(value).unwrap();

        assert_eq!(decoded, doc);
    }
}
